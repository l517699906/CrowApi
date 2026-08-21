pub mod handlers;
mod catalog;
mod protocol;
mod session;
mod transport;
mod tools;

use super::{Service, ServiceIssue, ServiceStatus};
use self::catalog::get_tool_summaries;
use crate::server::router::SharedState;
use crate::AppState;
use async_trait::async_trait;
use axum::{extract::DefaultBodyLimit, Router};
use std::sync::Arc;

pub struct McpService;

#[async_trait]
impl Service for McpService {
    fn id(&self) -> &'static str {
        "mcp"
    }
    fn name(&self) -> &'static str {
        "MCP Server"
    }
    fn description(&self) -> &'static str {
        "Model Context Protocol Server，对外暴露 RAG 工具（支持创建/更新/删除 RAG、上传/删除文档、导入源、构建索引、搜索、RAG问答）"
    }

    async fn status(&self, state: &Arc<AppState>) -> ServiceStatus {
        let pool = &state.db.pool;
        let gateway_running = state
            .server_running
            .load(std::sync::atomic::Ordering::SeqCst);
        let counts = sqlx::query_as::<_, (i64, i64)>(
            "SELECT
                (SELECT COUNT(*) FROM kb_knowledge_bases WHERE status = 1 AND mcp_enabled = 1),
                (SELECT COUNT(*) FROM wiki_projects WHERE status = 1 AND mcp_enabled = 1)",
        )
        .fetch_one(pool)
        .await;
        let mut issues = Vec::new();
        if !gateway_running {
            issues.push(ServiceIssue::new("GATEWAY_STOPPED", "网关服务未启动", true));
        }
        let (kb_count, wiki_count) = match counts {
            Ok(counts) => counts,
            Err(error) => {
                tracing::error!(%error, "MCP service health query failed");
                issues.push(ServiceIssue::new(
                    "DATABASE_UNAVAILABLE",
                    "MCP 服务数据库不可用",
                    true,
                ));
                (0, 0)
            }
        };
        let database_ok = !issues.iter().any(|issue| issue.code == "DATABASE_UNAVAILABLE");

        ServiceStatus {
            id: self.id().to_string(),
            name: self.name().to_string(),
            description: self.description().to_string(),
            enabled: true,
            running: gateway_running && database_ok,
            health: if !gateway_running || !database_ok {
                "unavailable"
            } else if issues.is_empty() {
                "healthy"
            } else {
                "degraded"
            }.to_string(),
            issues,
            stats: serde_json::json!({
                "available_knowledge_bases": kb_count,
                "available_wikis": wiki_count,
                "tools": get_tool_summaries(),
            }),
        }
    }

    fn routes(&self, _state: Arc<AppState>) -> Router<SharedState> {
        Router::new()
            // Primary JSON-RPC endpoint with the legacy SSE transport kept for compatibility.
            .route(
                "/mcp",
                axum::routing::post(handlers::handle_mcp)
                    .get(handlers::handle_mcp_sse)
                    .delete(handlers::handle_mcp_delete),
            )
            // Trailing-slash variant — some clients send /mcp/
            .route(
                "/mcp/",
                axum::routing::post(handlers::handle_mcp)
                    .get(handlers::handle_mcp_sse)
                    .delete(handlers::handle_mcp_delete),
            )
            // Legacy SSE endpoint — keep for backwards compat
            .route(
                "/mcp/sse",
                axum::routing::get(handlers::handle_mcp_sse).post(handlers::handle_mcp),
            )
            .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
    }
}
