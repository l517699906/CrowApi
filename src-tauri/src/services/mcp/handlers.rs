use crate::core::access::ACCESS_SCOPE_MCP_WRITE;
use crate::server::auth::AuthenticatedPrincipal;
use crate::server::router::SharedState;

use super::catalog::{get_tools, mcp_tool_requires_write, MCP_INSTRUCTIONS};
use super::protocol::{McpRequest, McpResponse};
use super::tools::handle_tool_call;
pub use super::transport::{handle_mcp, handle_mcp_delete, handle_mcp_sse};

pub(crate) const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05"];

pub(crate) fn negotiate_protocol_version(params: &serde_json::Value) -> &'static str {
    params
        .get("protocolVersion")
        .and_then(serde_json::Value::as_str)
        .and_then(|requested| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|supported| *supported == requested)
        })
        .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0])
}
// ── Core JSON-RPC dispatch ────────────────────────────────────────

/// Main MCP JSON-RPC handler — async dispatch
pub(crate) async fn dispatch_jsonrpc_async(
    shared: &SharedState,
    principal: &AuthenticatedPrincipal,
    req: &McpRequest,
) -> McpResponse {
    match req.method.as_str() {
        "initialize" => McpResponse::success(
            req.id.clone(),
            serde_json::json!({
                "protocolVersion": negotiate_protocol_version(&req.params),
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "CrowAPI Knowledge Base",
                    "version": "0.1.0"
                },
                "instructions": MCP_INSTRUCTIONS
            }),
        ),
        "notifications/initialized" => McpResponse::success(req.id.clone(), serde_json::json!({})),
        "tools/list" => McpResponse::success(
            req.id.clone(),
            serde_json::json!({
                "tools": get_tools(principal)
            }),
        ),
        "tools/call" => {
            let Some(tool_name) = req
                .params
                .get("name")
                .and_then(|n| n.as_str())
                .filter(|name| !name.is_empty())
            else {
                return McpResponse::error(
                    req.id.clone(),
                    -32602,
                    "Missing tool name".to_string(),
                );
            };

            let args = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            if !args.is_object() {
                return McpResponse::error(
                    req.id.clone(),
                    -32602,
                    "Tool arguments must be an object".to_string(),
                );
            }

            if mcp_tool_requires_write(tool_name)
                && !principal.allows(ACCESS_SCOPE_MCP_WRITE)
            {
                return McpResponse::error(
                    req.id.clone(),
                    -32003,
                    "MCP write scope is required for this tool".to_string(),
                );
            }

            match handle_tool_call(shared, tool_name, &args).await {
                Ok(result) => McpResponse::success(req.id.clone(), result),
                Err(e) => McpResponse::error(req.id.clone(), -32603, e),
            }
        }
        "ping" => McpResponse::success(req.id.clone(), serde_json::json!({})),
        _ => McpResponse::error(
            req.id.clone(),
            -32601,
            format!("Unknown method: {}", req.method),
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::super::catalog::{get_tool_summaries, get_tools, mcp_tool_requires_write};
    use super::super::protocol::{validate_jsonrpc_request, McpRequest};
    use super::super::session::{
        register_sse_session, remove_sse_session, session_sender_for_principal,
        SessionAccessError,
    };
    use crate::db::models::ApiKey;
    use crate::server::auth::AuthenticatedPrincipal;
    use tokio::sync::mpsc;

    fn principal(scopes: &[&str]) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal::new(
            ApiKey {
                id: "key-1".to_string(),
                name: "test".to_string(),
                key: "redacted:key-1".to_string(),
                key_lookup: None,
                key_hash: None,
                key_prefix: "sk-crowapi-test".to_string(),
                key_last4: "test".to_string(),
                status: 1,
                allowed_models: "[]".to_string(),
                allowed_channels: "[]".to_string(),
                access_scopes: serde_json::to_string(scopes).unwrap(),
                quota_limit: 0,
                quota_used: 0,
                expires_at: None,
                created_at: "now".to_string(),
                updated_at: "now".to_string(),
            },
            scopes.iter().map(|scope| (*scope).to_string()).collect(),
        )
    }

    #[test]
    fn jsonrpc_version_and_method_are_required() {
        let wrong_version = McpRequest {
            jsonrpc: "1.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "ping".to_string(),
            params: serde_json::json!({}),
        };
        let missing_method = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(2)),
            method: " ".to_string(),
            params: serde_json::json!({}),
        };

        assert!(validate_jsonrpc_request(&wrong_version).is_err());
        assert!(validate_jsonrpc_request(&missing_method).is_err());
    }

    #[test]
    fn initialize_only_advertises_the_implemented_protocol_version() {
        for requested in ["2024-11-05", "2025-03-26", "2025-06-18", "unknown"] {
            assert_eq!(
                super::negotiate_protocol_version(&serde_json::json!({
                    "protocolVersion": requested
                })),
                "2024-11-05"
            );
        }
        assert_eq!(
            super::negotiate_protocol_version(&serde_json::json!({})),
            "2024-11-05"
        );
    }

    #[test]
    fn mcp_write_tools_are_classified_before_dispatch() {
        assert!(mcp_tool_requires_write("create_knowledge_base"));
        assert!(mcp_tool_requires_write("save_wiki_page"));
        assert!(mcp_tool_requires_write("retry_background_task"));
        assert!(!mcp_tool_requires_write("search_knowledge_base"));
        assert!(!mcp_tool_requires_write("list_wiki_projects"));
        assert!(!mcp_tool_requires_write("get_background_task"));
    }

    #[test]
    fn tools_list_only_advertises_capabilities_the_principal_can_call() {
        let read_names = get_tools(&principal(&["mcp:read"]))
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        assert!(read_names.contains(&"search_knowledge_base".to_string()));
        assert!(!read_names.contains(&"create_knowledge_base".to_string()));
        assert!(!read_names.contains(&"save_wiki_page".to_string()));

        let write_names = get_tools(&principal(&["mcp:write"]))
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        assert!(write_names.contains(&"search_knowledge_base".to_string()));
        assert!(write_names.contains(&"create_knowledge_base".to_string()));
        assert!(write_names.contains(&"save_wiki_page".to_string()));
    }

    #[test]
    fn service_tool_summaries_are_derived_from_the_protocol_catalog() {
        let write_names = get_tools(&principal(&["mcp:write"]))
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<std::collections::HashSet<_>>();
        let summaries = get_tool_summaries();

        assert_eq!(summaries.len(), write_names.len());
        assert!(summaries.iter().all(|summary| {
            summary["name"]
                .as_str()
                .is_some_and(|name| write_names.contains(name))
        }));
    }

    #[tokio::test]
    async fn sse_sessions_are_bound_to_the_authenticated_principal() {
        let session_id = uuid::Uuid::new_v4().to_string();
        let (sender, _receiver) = mpsc::unbounded_channel();
        register_sse_session(session_id.clone(), sender, "key-1".to_string())
            .await
            .expect("register test MCP session");

        assert!(session_sender_for_principal(&session_id, "key-1").await.is_ok());
        assert_eq!(
            session_sender_for_principal(&session_id, "key-2")
                .await
                .expect_err("different principal must not reuse MCP session"),
            SessionAccessError::PrincipalMismatch,
        );
        remove_sse_session(&session_id).await;
        assert_eq!(
            session_sender_for_principal(&session_id, "key-1")
                .await
                .expect_err("removed MCP session must stay unavailable"),
            SessionAccessError::NotFound,
        );
    }
}
