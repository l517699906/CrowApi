pub mod models;
pub mod repository;
pub mod parser;
pub mod code_parser;
pub mod splitter;
pub mod embedder;
pub mod index;
pub mod retriever;
pub mod rag;
pub mod processor;
pub mod handlers;
pub mod routes;
pub mod importer;

use async_trait::async_trait;
use axum::Router;
use std::path::{Component, Path};
use std::sync::Arc;
use crate::AppState;
use crate::server::router::SharedState;
use super::{Service, ServiceIssue, ServiceStatus};

pub struct KnowledgeService;

pub(crate) fn safe_path_component<'a>(value: &'a str, label: &str) -> Result<&'a str, String> {
    let mut components = Path::new(value).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) if !value.is_empty() => Ok(value),
        _ => Err(format!("Invalid {}", label)),
    }
}

#[async_trait]
impl Service for KnowledgeService {
    fn id(&self) -> &'static str { "knowledge" }
    fn name(&self) -> &'static str { "RAG" }
    fn description(&self) -> &'static str { "本地 RAG 知识库：创建私有知识库，上传文档自动向量化并构建 HNSW 索引，通过 MCP 协议对外提供检索和 RAG 问答工具，支持任意 AI Agent 对接" }

    async fn status(&self, state: &Arc<AppState>) -> ServiceStatus {
        let pool = &state.db.pool;
        let gateway_running = state
            .server_running
            .load(std::sync::atomic::Ordering::SeqCst);
        let counts = sqlx::query_as::<_, (i64, i64, i64, i64)>(
            "SELECT
                (SELECT COUNT(*) FROM kb_knowledge_bases),
                (SELECT COUNT(*) FROM kb_documents),
                (SELECT COUNT(*) FROM kb_chunks),
                (SELECT COUNT(*) FROM channels WHERE status = 1)",
        )
        .fetch_one(pool)
        .await;
        let mut issues = Vec::new();
        if !gateway_running {
            issues.push(ServiceIssue::new(
                "GATEWAY_STOPPED",
                "网关服务未启动",
                true,
            ));
        }
        let (kb_count, doc_count, chunk_count, channel_count) = match counts {
            Ok(counts) => counts,
            Err(error) => {
                tracing::error!(%error, "knowledge service health query failed");
                issues.push(ServiceIssue::new(
                    "DATABASE_UNAVAILABLE",
                    "知识库数据库不可用",
                    true,
                ));
                (0, 0, 0, 0)
            }
        };
        if channel_count == 0 {
            issues.push(ServiceIssue::new(
                "AI_CHANNEL_UNAVAILABLE",
                "没有可用的 AI 渠道",
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
                "knowledge_bases": kb_count,
                "documents": doc_count,
                "chunks": chunk_count,
                "active_channels": channel_count,
            }),
        }
    }

    fn routes(&self, state: Arc<AppState>) -> Router<SharedState> {
        routes::create_router(state)
        }
}

#[cfg(test)]
mod tests {
    use super::safe_path_component;

    #[test]
    fn knowledge_file_components_reject_path_traversal() {
        assert!(safe_path_component("document.md", "filename").is_ok());
        for value in ["../document.md", "nested/document.md", "/tmp/document.md", "", "."] {
            assert!(safe_path_component(value, "filename").is_err(), "accepted unsafe component: {value}");
        }
    }
}
