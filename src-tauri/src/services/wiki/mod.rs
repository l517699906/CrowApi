pub mod handlers;
pub mod ingest;
pub mod models;
pub mod project;
pub mod repository;
pub mod routes;

use crate::services::{Service, ServiceIssue, ServiceStatus};
use crate::AppState;
use async_trait::async_trait;
use axum::Router;
use std::sync::Arc;

pub struct WikiService;

#[async_trait]
impl Service for WikiService {
    fn id(&self) -> &'static str {
        "wiki"
    }

    fn name(&self) -> &'static str {
        "Wiki"
    }

    fn description(&self) -> &'static str {
        "LLM 增量 RAG：自动摄入文档 → 生成结构化 Wiki 页面 → 知识图谱 → 深度研究 → MCP 工具暴露"
    }

    async fn status(&self, state: &Arc<AppState>) -> ServiceStatus {
        let pool = &state.db.pool;
        let gateway_running = state
            .server_running
            .load(std::sync::atomic::Ordering::SeqCst);
        let counts = sqlx::query_as::<_, (i64, i64, i64, i64)>(
            "SELECT
                (SELECT COUNT(*) FROM wiki_projects WHERE status = 1),
                (SELECT COUNT(*) FROM wiki_pages WHERE status = 'active'),
                (SELECT COUNT(*) FROM wiki_sources WHERE status = 'ingested'),
                (SELECT COUNT(*) FROM channels WHERE status = 1)",
        )
        .fetch_one(pool)
        .await;
        let mut issues = Vec::new();
        if !gateway_running {
            issues.push(ServiceIssue::new("GATEWAY_STOPPED", "网关服务未启动", true));
        }
        let (project_count, page_count, source_count, channel_count) = match counts {
            Ok(counts) => counts,
            Err(error) => {
                tracing::error!(%error, "Wiki service health query failed");
                issues.push(ServiceIssue::new(
                    "DATABASE_UNAVAILABLE",
                    "Wiki 数据库不可用",
                    true,
                ));
                (0, 0, 0, 0)
            }
        };
        if channel_count == 0 {
            issues.push(ServiceIssue::new(
                "AI_CHANNEL_UNAVAILABLE",
                "没有可用于 Wiki 摄入和问答的 AI 渠道",
                true,
            ));
        }
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
                "projects": project_count,
                "pages": page_count,
                "sources": source_count,
                "active_channels": channel_count,
            }),
        }
    }

    fn routes(&self, state: Arc<AppState>) -> Router<crate::server::router::SharedState> {
        routes::create_router(state)
    }
}
