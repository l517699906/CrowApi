pub mod handlers;

use super::{Service, ServiceIssue, ServiceStatus};
use crate::server::router::SharedState;
use crate::AppState;
use async_trait::async_trait;
use axum::Router;
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
                "tools": [
                {"name": "search_knowledge_base", "label": "语义搜索", "desc": "基于 HNSW 向量索引的语义搜索，返回匹配文本片段及相似度评分"},
                {"name": "list_knowledge_bases", "label": "列出 RAG", "desc": "列出所有已启用 MCP 的 RAG，含 ID、名称、文档数、切片数"},
                {"name": "read_document", "label": "读取文档", "desc": "读取 RAG 中指定文档的完整内容"},
                {"name": "ask_knowledge_base", "label": "RAG 问答", "desc": "基于 RAG 内容的智能问答，返回 AI 生成的回答及来源引用"},
                {"name": "get_knowledge_base_stats", "label": "RAG 统计", "desc": "获取 RAG 详细统计：文档数、切片数、Token 数、索引状态"},
                {"name": "create_knowledge_base", "label": "创建 RAG", "desc": "创建新 RAG，支持自定义名称、描述、向量模型"},
                {"name": "update_knowledge_base", "label": "更新 RAG", "desc": "更新 RAG 配置：名称、描述、分块大小、MCP 开关等"},
                {"name": "delete_knowledge_base", "label": "删除 RAG", "desc": "永久删除 RAG 及其所有文档、切片和索引"},
                {"name": "upload_document", "label": "上传文档", "desc": "上传文档到 RAG，自动解析→分块→向量化→索引"},
                {"name": "delete_document", "label": "删除文档", "desc": "删除 RAG 中的指定文档及其切片和向量"},
                {"name": "list_documents", "label": "文档列表", "desc": "列出 RAG 中所有文档及处理状态、切片数、Token 数"},
                {"name": "build_index", "label": "构建索引", "desc": "构建或重建 HNSW 向量索引，提升搜索性能"},
                {"name": "import_source", "label": "导入源", "desc": "从 Git 仓库、URL 或本地目录批量导入文档"}
            ],
            }),
        }
    }

    fn routes(&self, _state: Arc<AppState>) -> Router<SharedState> {
        Router::new()
            // Primary Streamable HTTP endpoint (POST = JSON-RPC, GET = SSE upgrade)
            .route(
                "/mcp",
                axum::routing::post(handlers::handle_mcp).get(handlers::handle_mcp_sse),
            )
            // Trailing-slash variant — some clients send /mcp/
            .route(
                "/mcp/",
                axum::routing::post(handlers::handle_mcp).get(handlers::handle_mcp_sse),
            )
            // Legacy SSE endpoint — keep for backwards compat
            .route(
                "/mcp/sse",
                axum::routing::get(handlers::handle_mcp_sse).post(handlers::handle_mcp),
            )
    }
}
