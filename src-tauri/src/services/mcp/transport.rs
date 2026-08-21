use super::handlers::dispatch_jsonrpc_async;
use super::protocol::{validate_jsonrpc_request, McpRequest, McpResponse};
use super::session::{
    register_sse_session, remove_sse_session, session_sender_for_principal,
    SessionAccessError, SessionGuard, SSE_SESSION_TTL,
};
use crate::server::auth::AuthenticatedPrincipal;
use crate::server::router::SharedState;
use axum::{
    body::Body,
    extract::{Extension, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use tokio::sync::mpsc;

// 3. Client POSTs JSON-RPC requests to that URL
// 4. Server pushes responses back through the SSE stream

pub async fn handle_mcp_sse(
    State(_shared): State<SharedState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Response {
    // Generate unique session ID
    let session_id = uuid::Uuid::new_v4().to_string();

    // Create channel for this session
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    if register_sse_session(session_id.clone(), tx, principal.api_key.id).await.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(McpResponse::error(
                None,
                -32000,
                "Too many active MCP sessions".to_string(),
            )),
        )
            .into_response();
    }

    // Build SSE stream
    let session_id_clone = session_id.clone();
    let stream = async_stream::stream! {
        let _session_guard = SessionGuard(session_id_clone.clone());

        // 1. Send endpoint event — tells client where to POST JSON-RPC
        let endpoint_url = format!("/mcp?session_id={}", session_id_clone);
        let endpoint_event = format!(
            "event: endpoint\ndata: {}\n\n",
            endpoint_url
        );
        yield Ok::<_, std::io::Error>(endpoint_event.into_bytes());

        // 2. Keep-alive loop + forward JSON-RPC responses
        let mut keepalive_interval = tokio::time::interval(std::time::Duration::from_secs(15));
        keepalive_interval.tick().await; // first tick is immediate
        let session_deadline = tokio::time::sleep(SSE_SESSION_TTL);
        tokio::pin!(session_deadline);

        loop {
            tokio::select! {
                // Forward JSON-RPC responses to client
                Some(msg) = rx.recv() => {
                    let sse_data = format!("data: {}\n\n", msg);
                    yield Ok::<_, std::io::Error>(sse_data.into_bytes());
                }
                // Keepalive
                _ = keepalive_interval.tick() => {
                    yield Ok::<_, std::io::Error>(b": keepalive\n\n".to_vec());
                }
                _ = &mut session_deadline => {
                    break;
                }
            }
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header("mcp-session-id", &session_id)
        .body(Body::from_stream(stream))
        .unwrap()
}

// ── POST endpoint: POST /mcp?session_id=xxx ───────────────────────
// Receives JSON-RPC requests and pushes responses through the SSE stream

#[derive(Debug, Deserialize)]
pub struct McpQueryParams {
    #[serde(default)]
    pub session_id: Option<String>,
}

fn session_id_from_request(headers: &HeaderMap, params: &McpQueryParams) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            params
                .session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

pub async fn handle_mcp(
    State(shared): State<SharedState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Query(params): Query<McpQueryParams>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let body_str = String::from_utf8_lossy(&body);

    // Parse JSON-RPC request
    let req: McpRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(McpResponse::error(None, -32700, "Parse error".to_string())),
            )
                .into_response();
        }
    };

    if let Err(response) = validate_jsonrpc_request(&req) {
        return (StatusCode::BAD_REQUEST, Json(response)).into_response();
    }

    let session_id = session_id_from_request(&headers, &params);
    let session_sender = if let Some(session_id) = session_id.as_deref() {
        match session_sender_for_principal(session_id, &principal.api_key.id).await {
            Ok(sender) => Some(sender),
            Err(SessionAccessError::NotFound) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(McpResponse::error(
                        req.id.clone(),
                        -32001,
                        "MCP session not found".to_string(),
                    )),
                )
                    .into_response();
            }
            Err(SessionAccessError::PrincipalMismatch) => {
                let trace_id = crate::server::auth::audit_authorization_denied(
                    &shared,
                    &principal,
                    "POST",
                    "/mcp",
                    headers
                        .get(header::ORIGIN)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    "MCP_SESSION_PRINCIPAL_MISMATCH",
                )
                .await;
                let mut response = (
                    StatusCode::FORBIDDEN,
                    Json(McpResponse::error(
                        req.id.clone(),
                        -32003,
                        "MCP session belongs to another API key".to_string(),
                    )),
                )
                    .into_response();
                if let Ok(value) = HeaderValue::from_str(&trace_id) {
                    response
                        .headers_mut()
                        .insert("x-crowapi-trace-id", value);
                }
                return response;
            }
        }
    } else {
        None
    };

    // Check if this is a notification (no id → no response)
    let is_notification = req.id.is_none();

    let response = dispatch_jsonrpc_async(&shared, &principal, &req).await;

    // If session_id is provided, push non-notification responses through SSE.
    if let Some(sender) = session_sender {
        if !is_notification && sender.send(response.to_json_string()).is_err() {
            if let Some(session_id) = session_id.as_deref() {
                remove_sse_session(session_id).await;
            }
            return (
                StatusCode::GONE,
                Json(McpResponse::error(
                    req.id.clone(),
                    -32001,
                    "MCP session is closed".to_string(),
                )),
            )
                .into_response();
        }
    }

    // For SSE transport: return 202 Accepted (response goes through SSE)
    // For direct POST (no session_id): return JSON response directly
    if session_id.is_some() {
        if is_notification {
            return StatusCode::ACCEPTED.into_response();
        }
        // Response is sent via SSE, but also return 202
        return StatusCode::ACCEPTED.into_response();
    }

    // No session_id — notifications get 202 with no body
    if is_notification {
        return StatusCode::ACCEPTED.into_response();
    }

    // Direct POST: return the JSON-RPC response body.
    Json(response).into_response()
}

pub async fn handle_mcp_delete(
    State(shared): State<SharedState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Query(params): Query<McpQueryParams>,
    headers: HeaderMap,
) -> Response {
    let Some(session_id) = session_id_from_request(&headers, &params) else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    match session_sender_for_principal(&session_id, &principal.api_key.id).await {
        Ok(_) => {
            remove_sse_session(&session_id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(SessionAccessError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(SessionAccessError::PrincipalMismatch) => {
            let trace_id = crate::server::auth::audit_authorization_denied(
                &shared,
                &principal,
                "DELETE",
                "/mcp",
                headers
                    .get(header::ORIGIN)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
                "MCP_SESSION_PRINCIPAL_MISMATCH",
            )
            .await;
            let mut response = StatusCode::FORBIDDEN.into_response();
            if let Ok(value) = HeaderValue::from_str(&trace_id) {
                response
                    .headers_mut()
                    .insert("x-crowapi-trace-id", value);
            }
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{session_id_from_request, McpQueryParams};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn session_header_takes_precedence_over_query_parameter() {
        let mut headers = HeaderMap::new();
        headers.insert("mcp-session-id", HeaderValue::from_static("  header-session  "));
        let params = McpQueryParams {
            session_id: Some("query-session".to_string()),
        };

        assert_eq!(
            session_id_from_request(&headers, &params).as_deref(),
            Some("header-session")
        );
    }

    #[test]
    fn query_parameter_is_trimmed_when_header_is_missing_or_empty() {
        let params = McpQueryParams {
            session_id: Some("  query-session  ".to_string()),
        };
        assert_eq!(
            session_id_from_request(&HeaderMap::new(), &params).as_deref(),
            Some("query-session")
        );

        let mut headers = HeaderMap::new();
        headers.insert("mcp-session-id", HeaderValue::from_static("   "));
        assert_eq!(
            session_id_from_request(&headers, &params).as_deref(),
            Some("query-session")
        );
    }

    #[test]
    fn empty_session_identifiers_are_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert("mcp-session-id", HeaderValue::from_static("   "));
        let params = McpQueryParams {
            session_id: Some("   ".to_string()),
        };

        assert_eq!(session_id_from_request(&headers, &params), None);
    }
}
