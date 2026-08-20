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
use super::{Service, ServiceStatus};

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
        let kb_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kb_knowledge_bases")
            .fetch_one(pool).await.unwrap_or(0);
        let doc_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kb_documents")
            .fetch_one(pool).await.unwrap_or(0);
        let chunk_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kb_chunks")
            .fetch_one(pool).await.unwrap_or(0);

        ServiceStatus {
            id: self.id().to_string(),
            name: self.name().to_string(),
            description: self.description().to_string(),
            enabled: true,
            running: true,
            stats: serde_json::json!({
                "knowledge_bases": kb_count,
                "documents": doc_count,
                "chunks": chunk_count,
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
