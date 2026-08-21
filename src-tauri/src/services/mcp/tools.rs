use crate::db::repository::Repository;
use crate::server::router::SharedState;
use crate::services::knowledge::{
    embedder, lifecycle, rag, repository::KbRepository, retriever,
};
use crate::services::wiki::{
    handlers as wiki_handlers, ingest as wiki_ingest, project as wiki_project,
    repository::WikiRepository,
};
use sqlx::SqlitePool;
use tauri::Manager;
// ── Tool call handlers ────────────────────────────────────────────

async fn ensure_mcp_kb_access(pool: &SqlitePool, kb_id: &str) -> Result<(), String> {
    let kb = KbRepository::new(pool.clone())
        .get_kb(kb_id)
        .await
        .map_err(|_| "Knowledge base not found".to_string())?;
    if kb.mcp_enabled != 1 {
        return Err("Knowledge base is not exposed through MCP".to_string());
    }
    Ok(())
}

async fn ensure_mcp_task_access(
    pool: &SqlitePool,
    task: &crate::services::tasks::models::BackgroundTask,
) -> Result<(), String> {
    match task.domain.as_str() {
        "knowledge" => ensure_mcp_kb_access(pool, &task.resource_id).await,
        "wiki" => {
            let project = WikiRepository::new(pool.clone())
                .get_project(&task.resource_id)
                .await?;
            if project.mcp_enabled == 1 {
                Ok(())
            } else {
                Err("Wiki project is not exposed through MCP".to_string())
            }
        }
        _ => Err("Task domain is not exposed through MCP".to_string()),
    }
}

pub(crate) async fn handle_tool_call(
    shared: &SharedState,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let pool = &shared.state.db.pool;

    const KB_SCOPED_TOOLS: &[&str] = &[
        "read_document",
        "get_knowledge_base_stats",
        "update_knowledge_base",
        "delete_knowledge_base",
        "delete_document",
        "list_documents",
        "build_index",
        "import_source",
    ];
    if KB_SCOPED_TOOLS.contains(&tool_name) {
        let kb_id = args
            .get("kb_id")
            .and_then(|value| value.as_str())
            .ok_or("Missing kb_id")?;
        ensure_mcp_kb_access(pool, kb_id).await?;
    }
    if matches!(tool_name, "search_knowledge_base" | "ask_knowledge_base") {
        if let Some(kb_id) = args
            .get("kb_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        {
            ensure_mcp_kb_access(pool, kb_id).await?;
        }
    }

    const PROJECT_SCOPED_WIKI_TOOLS: &[&str] = &[
        "get_wiki_project",
        "list_wiki_pages",
        "get_wiki_page",
        "save_wiki_page",
        "search_wiki",
        "ask_wiki",
        "get_wiki_tags",
        "get_wiki_graph",
        "list_wiki_sources",
        "ingest_wiki_source",
        "delete_wiki_project",
        "delete_wiki_page",
        "add_wiki_source",
        "delete_wiki_source",
    ];
    if PROJECT_SCOPED_WIKI_TOOLS.contains(&tool_name) {
        let project_id = args
            .get("project_id")
            .and_then(|value| value.as_str())
            .ok_or("Missing project_id")?;
        let project = WikiRepository::new(pool.clone())
            .get_project(project_id)
            .await?;
        if project.mcp_enabled != 1 {
            return Err("Wiki project is not exposed through MCP".to_string());
        }
    }

    match tool_name {
        "list_background_tasks" => {
            let domain = args
                .get("domain")
                .and_then(|value| value.as_str())
                .ok_or("Missing domain")?;
            let resource_id = args
                .get("resource_id")
                .and_then(|value| value.as_str())
                .ok_or("Missing resource_id")?;
            match domain {
                "knowledge" => ensure_mcp_kb_access(pool, resource_id).await?,
                "wiki" => {
                    let project = WikiRepository::new(pool.clone())
                        .get_project(resource_id)
                        .await?;
                    if project.mcp_enabled != 1 {
                        return Err("Wiki project is not exposed through MCP".to_string());
                    }
                }
                _ => return Err("Unsupported task domain".to_string()),
            }
            let tasks = crate::services::tasks::repository::TaskRepository::new(pool.clone())
                .list(&crate::services::tasks::models::TaskListFilter {
                    domain: Some(domain.to_string()),
                    resource_id: Some(resource_id.to_string()),
                    status: args
                        .get("status")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    limit: args
                        .get("limit")
                        .and_then(|value| value.as_i64())
                        .map(|value| value.clamp(1, 100)),
                    ..Default::default()
                })
                .await
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&tasks).map_err(|error| error.to_string())?
                }],
                "isError": false
            }))
        }

        "get_background_task" => {
            let task_id = args
                .get("task_id")
                .and_then(|value| value.as_str())
                .ok_or("Missing task_id")?;
            let task = crate::services::tasks::repository::TaskRepository::new(pool.clone())
                .get(task_id)
                .await
                .map_err(|error| error.to_string())?;
            ensure_mcp_task_access(pool, &task).await?;
            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&task).map_err(|error| error.to_string())?
                }],
                "isError": false
            }))
        }

        "cancel_background_task" => {
            let task_id = args
                .get("task_id")
                .and_then(|value| value.as_str())
                .ok_or("Missing task_id")?;
            let tasks = crate::services::tasks::repository::TaskRepository::new(pool.clone());
            let current = tasks.get(task_id).await.map_err(|error| error.to_string())?;
            ensure_mcp_task_access(pool, &current).await?;
            let task = tasks
                .request_cancel(task_id)
                .await
                .map_err(|error| error.to_string())?;
            crate::services::tasks::emit_task_event(
                &shared.app,
                &task,
                Some("MCP 已请求取消任务"),
            );
            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&task).map_err(|error| error.to_string())?
                }],
                "isError": false
            }))
        }

        "retry_background_task" => {
            let task_id = args
                .get("task_id")
                .and_then(|value| value.as_str())
                .ok_or("Missing task_id")?;
            let tasks = crate::services::tasks::repository::TaskRepository::new(pool.clone());
            let current = tasks.get(task_id).await.map_err(|error| error.to_string())?;
            ensure_mcp_task_access(pool, &current).await?;
            let task = crate::services::tasks::dispatcher::retry_and_dispatch(
                pool,
                &shared.app,
                task_id,
            )
            .await?;
            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&task).map_err(|error| error.to_string())?
                }],
                "isError": false
            }))
        }

        "search_knowledge_base" => {
            let query = args
                .get("query")
                .and_then(|q| q.as_str())
                .ok_or("Missing query")?;
            let kb_id = args.get("kb_id").and_then(|k| k.as_str()).unwrap_or("");
            let top_k = args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(5) as usize;
            let search_mode = args
                .get("search_mode")
                .and_then(|s| s.as_str())
                .unwrap_or("hybrid");
            let vector_weight = args
                .get("vector_weight")
                .and_then(|w| w.as_f64())
                .unwrap_or(0.7) as f32;
            let keyword_weight = args
                .get("keyword_weight")
                .and_then(|w| w.as_f64())
                .unwrap_or(0.3) as f32;

            let (emb_model, embedding_channel_id) = if !kb_id.is_empty() {
                let kb_repo = KbRepository::new(pool.clone());
                let kb = kb_repo
                    .get_kb(kb_id)
                    .await
                    .map_err(|error| format!("Failed to load knowledge base: {}", error))?;
                (
                    kb.embedding_model
                        .unwrap_or_else(|| "text-embedding-3-small".to_string()),
                    kb.embedding_channel_id,
                )
            } else {
                ("text-embedding-3-small".to_string(), None)
            };

            let repo = Repository::new(pool.clone());

            // Keyword-only mode: no embedding needed
            if search_mode == "keyword" && !kb_id.is_empty() {
                let results = retriever::keyword_only_search(pool, kb_id, query, top_k).await?;
                let content: Vec<serde_json::Value> = results.iter().map(|r| {
                    serde_json::json!({
                        "type": "text",
                        "text": format!("[{}] (score: {:.2}) [keyword]\n{}", r.filename, r.score, r.content)
                    })
                }).collect();
                return Ok(serde_json::json!({ "content": content, "isError": false }));
            }

            let embeddings = embedder::embed_with_channel(
                &[query.to_string()],
                &emb_model,
                &repo,
                embedding_channel_id.as_deref(),
            ).await?;
            if embeddings.is_empty() {
                return Err("Failed to embed query".to_string());
            }

            if kb_id.is_empty() {
                // Cross-KB search: always hybrid (search_all doesn't support mode selection)
                let results = retriever::search_all(pool, &embeddings[0], top_k, true).await?;
                let content: Vec<serde_json::Value> = results.iter().map(|r| {
                    serde_json::json!({
                        "type": "text",
                        "text": format!("[{}] (score: {:.2})\n{}", r.filename, r.score, r.content)
                    })
                }).collect();
                Ok(serde_json::json!({ "content": content, "isError": false }))
            } else {
                // Single-KB search with details
                let scored = retriever::hybrid_search_with_details(
                    pool,
                    kb_id,
                    query,
                    &embeddings[0],
                    top_k,
                    vector_weight,
                    keyword_weight,
                )
                .await?;

                let content: Vec<serde_json::Value> = scored
                    .iter()
                    .map(|s| {
                        let r = &s.result;
                        let mut line = format!("[{}] (score: {:.2}", r.filename, r.score);
                        if let Some(vs) = s.vector_score {
                            line.push_str(&format!(", vec: {:.2}", vs));
                        }
                        if let Some(ks) = s.keyword_score {
                            line.push_str(&format!(", kw: {:.2}", ks));
                        }
                        line.push_str(")\n");
                        line.push_str(&r.content);
                        serde_json::json!({ "type": "text", "text": line })
                    })
                    .collect();

                Ok(serde_json::json!({ "content": content, "isError": false }))
            }
        }

        "list_knowledge_bases" => {
            let kb_repo = KbRepository::new(pool.clone());
            let kbs = kb_repo.get_all_kbs().await.map_err(|e| e.to_string())?;

            // Only expose KBs with mcp_enabled = 1
            let exposed: Vec<_> = kbs.iter().filter(|kb| kb.mcp_enabled == 1).collect();

            let content: Vec<serde_json::Value> = exposed.iter().map(|kb| {
                serde_json::json!({
                    "type": "text",
                    "text": format!("ID: {}\nName: {}\nDocuments: {}\nChunks: {}\nDescription: {}",
                        kb.id, kb.name, kb.doc_count, kb.chunk_count,
                        kb.description.as_deref().unwrap_or("N/A"))
                })
            }).collect();

            Ok(serde_json::json!({
                "content": content,
                "isError": false
            }))
        }

        "read_document" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;
            let doc_id = args
                .get("doc_id")
                .and_then(|d| d.as_str())
                .ok_or("Missing doc_id")?;

            let kb_repo = KbRepository::new(pool.clone());
            let doc = kb_repo
                .get_document_in_kb(kb_id, doc_id)
                .await
                .map_err(|e| e.to_string())?;

            let content = if let Some(path) = &doc.file_path {
                std::fs::read_to_string(path).unwrap_or_else(|_| "Failed to read file".to_string())
            } else {
                "No file path available".to_string()
            };

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("File: {}\n\n{}", doc.filename, content)
                }],
                "isError": false
            }))
        }

        "ask_knowledge_base" => {
            let question = args
                .get("question")
                .and_then(|q| q.as_str())
                .ok_or("Missing question")?;
            let kb_id = args.get("kb_id").and_then(|k| k.as_str()).unwrap_or("");
            let top_k = args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(5) as usize;
            let search_mode = args
                .get("search_mode")
                .and_then(|s| s.as_str())
                .unwrap_or("hybrid");
            let vector_weight = args
                .get("vector_weight")
                .and_then(|w| w.as_f64())
                .unwrap_or(0.7) as f32;
            let keyword_weight = args
                .get("keyword_weight")
                .and_then(|w| w.as_f64())
                .unwrap_or(0.3) as f32;

            let emb_model = if !kb_id.is_empty() {
                let kb_repo = KbRepository::new(pool.clone());
                kb_repo
                    .get_kb(kb_id)
                    .await
                    .ok()
                    .and_then(|kb| kb.embedding_model)
                    .unwrap_or_else(|| "text-embedding-3-small".to_string())
            } else {
                "text-embedding-3-small".to_string()
            };

            // Auto-select chat model from available channels if not specified
            let chat_model = if let Some(m) = args.get("model").and_then(|m| m.as_str()) {
                m.to_string()
            } else {
                let main_repo = Repository::new(pool.clone());
                let channels = main_repo.get_enabled_channels().await.unwrap_or_default();
                let embedding_models = [
                    "text-embedding-3-small",
                    "text-embedding-3-large",
                    "text-embedding-ada-002",
                    "bge-large-zh",
                    "bge-small-zh",
                ];
                let mut picked: Option<String> = None;
                for ch in &channels {
                    let models: Vec<String> = serde_json::from_str(&ch.models).unwrap_or_default();
                    if let Some(m) = models
                        .iter()
                        .find(|m| !embedding_models.contains(&m.as_str()))
                    {
                        picked = Some(m.clone());
                        break;
                    }
                }
                picked.unwrap_or_else(|| "gpt-4o".to_string())
            };

            let answer = rag::ask_with_config(
                pool,
                kb_id,
                question,
                &emb_model,
                &chat_model,
                top_k,
                true,
                &[],
                &shared.app,
                vector_weight,
                keyword_weight,
                search_mode,
            )
            .await?;

            let mut content = vec![serde_json::json!({
                "type": "text",
                "text": answer.answer
            })];

            // Source citations
            for source in &answer.sources {
                content.push(serde_json::json!({
                    "type": "text",
                    "text": format!("Source: {} (score: {:.2})\n{}", source.filename, source.score, source.snippet)
                }));
            }

            // Retrieval details: per-chunk score breakdown
            if let Some(details) = &answer.retrieval_details {
                let mut detail_lines = String::from("\n--- Retrieval Details ---\n");
                for d in details {
                    let mut line = format!("• {} (score: {:.2}", d.filename, d.score);
                    if let Some(vs) = d.vector_score {
                        line.push_str(&format!(", vec: {:.2}", vs));
                    }
                    if let Some(ks) = d.keyword_score {
                        line.push_str(&format!(", kw: {:.2}", ks));
                    }
                    if let Some(sym) = &d.symbol_name {
                        line.push_str(&format!(", symbol: {}", sym));
                        if let Some(kind) = &d.symbol_kind {
                            line.push_str(&format!(" ({})", kind));
                        }
                    }
                    line.push_str(")");
                    detail_lines.push_str(&line);
                    detail_lines.push('\n');
                }
                content.push(serde_json::json!({
                    "type": "text",
                    "text": detail_lines
                }));
            }

            Ok(serde_json::json!({
                "content": content,
                "isError": false
            }))
        }

        "get_knowledge_base_stats" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;

            let kb_repo = KbRepository::new(pool.clone());
            let kb = kb_repo.get_kb(kb_id).await.map_err(|e| e.to_string())?;
            let docs = kb_repo
                .get_documents(kb_id)
                .await
                .map_err(|e| e.to_string())?;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Knowledge Base: {}\nDocuments: {} (ready: {})\nChunks: {}\nTotal Tokens: {}",
                        kb.name,
                        kb.doc_count,
                        docs.iter().filter(|d| d.status == "ready").count(),
                        kb.chunk_count,
                        kb.total_tokens
                    )
                }],
                "isError": false
            }))
        }

        // ── Write tools: Knowledge Base lifecycle ──────────────────
        "create_knowledge_base" => {
            let name = args
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or("Missing name")?;
            let description = args.get("description").and_then(|d| d.as_str());
            let embedding_model = args.get("embedding_model").and_then(|m| m.as_str());
            let embedding_channel_id = args.get("embedding_channel_id").and_then(|c| c.as_str());

            let input = crate::services::knowledge::models::CreateKbInput {
                name: name.to_string(),
                description: description.map(|s| s.to_string()),
                embedding_model: embedding_model.map(|s| s.to_string()),
                embedding_channel_id: embedding_channel_id.map(|s| s.to_string()),
            };

            let kb_repo = KbRepository::new(pool.clone());
            let kb = kb_repo.create_kb(&input).await.map_err(|e| e.to_string())?;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Knowledge base created successfully.\nID: {}\nName: {}\nDescription: {}\nEmbedding model: {}\nMCP enabled: true",
                        kb.id,
                        kb.name,
                        kb.description.as_deref().unwrap_or("N/A"),
                        kb.embedding_model.as_deref().unwrap_or("text-embedding-3-small")
                    )
                }],
                "isError": false
            }))
        }

        "update_knowledge_base" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;

            let input = crate::services::knowledge::models::UpdateKbInput {
                name: args
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string()),
                description: args
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string()),
                embedding_model: args
                    .get("embedding_model")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string()),
                embedding_channel_id: args
                    .get("embedding_channel_id")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string()),
                status: args.get("status").and_then(|s| s.as_i64()),
                mcp_enabled: args.get("mcp_enabled").and_then(|m| m.as_i64()),
                chunk_size: args.get("chunk_size").and_then(|c| c.as_i64()),
                chunk_overlap: args.get("chunk_overlap").and_then(|c| c.as_i64()),
                excluded_dirs: args
                    .get("excluded_dirs")
                    .and_then(|e| e.as_str())
                    .map(|s| s.to_string()),
                excluded_files: args
                    .get("excluded_files")
                    .and_then(|e| e.as_str())
                    .map(|s| s.to_string()),
                included_files: args
                    .get("included_files")
                    .and_then(|i| i.as_str())
                    .map(|s| s.to_string()),
                embedding_batch_size: args.get("embedding_batch_size").and_then(|b| b.as_i64()),
            };

            let kb_repo = KbRepository::new(pool.clone());
            let outcome = kb_repo
                .update_kb_with_effects(kb_id, &input)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(task_id) = outcome.reprocess_task_id.as_deref() {
                crate::services::tasks::dispatcher::dispatch_existing(
                    pool,
                    &shared.app,
                    task_id,
                )
                .await?;
            }
            let kb = outcome.knowledge_base;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Knowledge base updated.\nID: {}\nName: {}\nMCP enabled: {}\nChunk size: {}\nChunk overlap: {}",
                        kb.id, kb.name, kb.mcp_enabled, kb.chunk_size, kb.chunk_overlap
                    )
                }],
                "isError": false
            }))
        }

        "delete_knowledge_base" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;

            let outcome = lifecycle::delete_knowledge_base(pool, &shared.app, kb_id)
                .await
                .map_err(|error| error.to_string())?;
            let kb = outcome.knowledge_base;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Knowledge base '{}' ({}) has been permanently deleted.", kb.name, kb_id)
                }],
                "isError": false
            }))
        }

        // ── Write tools: Document management ───────────────────────
        "upload_document" => {
            let filename = args
                .get("filename")
                .and_then(|f| f.as_str())
                .ok_or("Missing filename")?;
            let content_b64 = args
                .get("content")
                .and_then(|c| c.as_str())
                .ok_or("Missing content")?;

            let kb_repo = KbRepository::new(pool.clone());

            // If kb_id not provided, return available KBs for user to choose
            let kb_id = match args.get("kb_id").and_then(|k| k.as_str()) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => {
                    let kbs = kb_repo.get_all_kbs().await.map_err(|e| e.to_string())?;
                    let exposed: Vec<_> = kbs.iter().filter(|kb| kb.mcp_enabled == 1).collect();

                    if exposed.is_empty() {
                        return Ok(serde_json::json!({
                            "content": [{
                                "type": "text",
                                "text": "⚠️ 未指定 RAG，且当前没有任何可用的知识库。\n\n请先调用 create_knowledge_base 创建一个 RAG，然后再上传文档。"
                            }],
                            "isError": false
                        }));
                    }

                    let mut lines = vec!["⚠️ 未指定目标 RAG。请选择一个已有知识库，或确认创建新库。\n\n已有知识库列表:".to_string()];
                    for (i, kb) in exposed.iter().enumerate() {
                        lines.push(format!(
                            "\n[{}] ID: {}\n    名称: {}\n    文档数: {} | 切片数: {} | Tokens: {}\n    描述: {}",
                            i + 1,
                            kb.id,
                            kb.name,
                            kb.doc_count,
                            kb.chunk_count,
                            kb.total_tokens,
                            kb.description.as_deref().unwrap_or("无")
                        ));
                    }
                    lines.push("\n\n请告诉 AI 你要上传到哪个 RAG（提供 ID 或名称），或者要求创建新 RAG。".to_string());

                    return Ok(serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": lines.join("\n")
                        }],
                        "isError": false
                    }));
                }
            };
            ensure_mcp_kb_access(pool, &kb_id).await?;

            let filename = filename.to_string();

            let content = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(content_b64)
                    .map_err(|e| format!("Invalid base64: {}", e))?
            };

            use sha2::Digest;
            let hash = sha2::Sha256::digest(&content);
            let hash_hex = hex::encode(hash);

            let kb_repo = KbRepository::new(pool.clone());

            // Check duplicate
            if let Ok(Some(_)) = kb_repo.find_document_by_hash(&kb_id, &hash_hex).await {
                return Ok(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Document '{}' already exists in this knowledge base (same content hash).", filename)
                    }],
                    "isError": false
                }));
            }

            let file_type = crate::services::knowledge::parser::get_file_type(&filename);
            let file_size = content.len() as i64;

            // Save file to disk
            let app_data_dir = shared
                .app
                .path()
                .app_data_dir()
                .map_err(|error| format!("Failed to locate application data directory: {}", error))?;
            let kb_dir = app_data_dir.join("kb_files").join(&kb_id);
            std::fs::create_dir_all(&kb_dir)
                .map_err(|error| format!("Failed to create knowledge base directory: {}", error))?;
            let doc_id = uuid::Uuid::new_v4().to_string();
            let file_path = kb_dir.join(format!("{}_{}", &doc_id, &filename));
            std::fs::write(&file_path, &content)
                .map_err(|error| format!("Failed to persist knowledge document: {}", error))?;
            let file_path_str = file_path.to_string_lossy().to_string();

            let doc = match kb_repo
                .create_document(
                    &kb_id,
                    &filename,
                    Some(&file_path_str),
                    &file_type,
                    file_size,
                    &hash_hex,
                )
                .await
            {
                Ok(document) => document,
                Err(error) => {
                    if let Err(cleanup_error) = std::fs::remove_file(&file_path) {
                        tracing::warn!(%cleanup_error, path = %file_path.display(), "failed to remove document after DB insert error");
                    }
                    return Err(error.to_string());
                }
            };

            let kb = kb_repo.get_kb(&kb_id).await.map_err(|e| e.to_string())?;
            let emb_model = kb.embedding_model.clone();

            let task_id = crate::services::knowledge::processor::start_document_processing(
                pool,
                &shared.app,
                &kb_id,
                &doc.id,
                filename.clone(),
                content,
                emb_model,
                None,
                true,
            )
            .await
            .map_err(|error| format!("Failed to start document processing: {}", error))?;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Document '{}' uploaded to knowledge base.\nDoc ID: {}\nTask ID: {}\nFile type: {}\nSize: {} bytes\nStatus: processing (will be automatically chunked, embedded, and indexed)",
                        filename, doc.id, task_id, file_type, file_size
                    )
                }],
                "isError": false
            }))
        }

        "delete_document" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;
            let doc_id = args
                .get("doc_id")
                .and_then(|d| d.as_str())
                .ok_or("Missing doc_id")?;

            let kb_repo = KbRepository::new(pool.clone());
            let doc = kb_repo
                .get_document_in_kb(kb_id, doc_id)
                .await
                .map_err(|e| e.to_string())?;

            kb_repo
                .delete_document(doc_id)
                .await
                .map_err(|e| e.to_string())?;
            crate::services::knowledge::storage::cleanup_document_files(
                &shared.app,
                std::slice::from_ref(&doc),
            )
            .await;
            if let Err(error) = crate::services::knowledge::retriever::schedule_index_build(
                pool,
                kb_id,
                &shared.app,
            )
            .await
            {
                tracing::warn!(%error, %kb_id, "failed to schedule index after MCP document deletion");
            }

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Document '{}' ({}) has been deleted from the knowledge base.", doc.filename, doc_id)
                }],
                "isError": false
            }))
        }

        "list_documents" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;

            let kb_repo = KbRepository::new(pool.clone());
            let docs = kb_repo
                .get_documents(kb_id)
                .await
                .map_err(|e| e.to_string())?;

            if docs.is_empty() {
                return Ok(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": "No documents in this knowledge base yet."
                    }],
                    "isError": false
                }));
            }

            let lines: Vec<String> = docs
                .iter()
                .map(|d| {
                    format!(
                        "- {} | ID: {} | Status: {} | Chunks: {} | Tokens: {} | Size: {} bytes",
                        d.filename, d.id, d.status, d.chunk_count, d.token_count, d.file_size
                    )
                })
                .collect();

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Documents in knowledge base ({} total):\n{}", docs.len(), lines.join("\n"))
                }],
                "isError": false
            }))
        }

        // ── Write tools: Index management ──────────────────────────
        "build_index" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;

            let task_id = crate::services::knowledge::retriever::start_index_build(
                pool,
                kb_id,
                &shared.app,
            )
            .await
            .map_err(|error| {
                if error == crate::services::knowledge::retriever::INDEX_BUILD_ALREADY_RUNNING {
                    "Index build is already running for this knowledge base".to_string()
                } else {
                    format!("Failed to start index build: {}", error)
                }
            })?;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Index build started for knowledge base {} (task {}). Use get_knowledge_base_stats to check progress.", kb_id, task_id)
                }],
                "isError": false
            }))
        }

        // ── Write tools: Source import ─────────────────────────────
        "import_source" => {
            let kb_id = args
                .get("kb_id")
                .and_then(|k| k.as_str())
                .ok_or("Missing kb_id")?;
            let source_type = args
                .get("source_type")
                .and_then(|s| s.as_str())
                .ok_or("Missing source_type")?;
            let kb_id = kb_id.to_string();

            let input =
                crate::services::knowledge::models::ImportSourceInput {
                    source_type: source_type.to_string(),
                    repo_url: args
                        .get("repo_url")
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string()),
                    branch: args
                        .get("branch")
                        .and_then(|b| b.as_str())
                        .map(|s| s.to_string()),
                    token: args
                        .get("token")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string()),
                    url: args
                        .get("url")
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string()),
                    dir_path: args
                        .get("dir_path")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string()),
                    excluded_dirs: args.get("excluded_dirs").and_then(|e| e.as_array()).map(
                        |arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        },
                    ),
                    included_files: args.get("included_files").and_then(|i| i.as_array()).map(
                        |arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        },
                    ),
                    max_file_size: args
                        .get("max_file_size")
                        .and_then(|m| m.as_u64())
                        .map(|v| v as usize),
                };

            let kb_repo = KbRepository::new(pool.clone());
            let source = kb_repo
                .create_source(
                    &kb_id,
                    &input.source_type,
                    input.repo_url.as_deref().or(input.url.as_deref()),
                    input.dir_path.as_deref(),
                    input.branch.as_deref(),
                )
                .await
                .map_err(|e| e.to_string())?;

            let task_id = crate::services::knowledge::importer::start_import_source(
                pool,
                &shared.app,
                &kb_id,
                source.clone(),
                input,
            )
            .await
            .map_err(|error| format!("Failed to start source import: {}", error))?;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Import started.\nSource ID: {}\nTask ID: {}\nType: {}\nKnowledge base: {}\nThe import runs asynchronously. Use get_background_task to check progress.",
                        source.id, task_id, source_type, kb_id
                    )
                }],
                "isError": false
            }))
        }

        // ── Wiki tools ─────────────────────────────────────────────
        "list_wiki_projects" => {
            let wiki_repo = WikiRepository::new(pool.clone());
            let projects = wiki_repo.list_projects().await.map_err(|e| e.to_string())?;

            let content: Vec<serde_json::Value> = projects.iter().filter(|p| p.mcp_enabled == 1).map(|p| {
                serde_json::json!({
                    "type": "text",
                    "text": format!("ID: {}\nName: {}\nPages: {} | Sources: {}\nDescription: {}",
                        p.id, p.name, p.page_count, p.source_count, p.description.as_deref().unwrap_or("N/A"))
                })
            }).collect();

            Ok(serde_json::json!({ "content": content, "isError": false }))
        }

        "get_wiki_project" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let wiki_repo = WikiRepository::new(pool.clone());
            let proj = wiki_repo.get_project(project_id).await.map_err(|e| e.to_string())?;
            let stats = wiki_repo.get_stats(project_id).await.unwrap_or(serde_json::json!({}));

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "ID: {}\nName: {}\nPages: {} | Sources: {}\nDescription: {}\nStats: {}",
                        proj.id, proj.name, proj.page_count, proj.source_count,
                        proj.description.as_deref().unwrap_or("N/A"),
                        serde_json::to_string_pretty(&stats).unwrap_or_default()
                    )
                }],
                "isError": false
            }))
        }

        "list_wiki_pages" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let wiki_repo = WikiRepository::new(pool.clone());
            let pages = wiki_repo.list_pages(project_id).await.map_err(|e| e.to_string())?;

            if pages.is_empty() {
                return Ok(serde_json::json!({
                    "content": [{ "type": "text", "text": "No wiki pages yet." }],
                    "isError": false
                }));
            }

            let lines: Vec<String> = pages.iter().map(|p| {
                format!("- {} ({}) | {}", p.title, p.path, p.page_type)
            }).collect();

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Wiki pages ({} total):\n{}", pages.len(), lines.join("\n"))
                }],
                "isError": false
            }))
        }

        "get_wiki_page" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let path = args.get("path").and_then(|s| s.as_str())
                .ok_or("Missing path")?;

            let content = wiki_project::read_page(project_id, path)
                .await
                .map_err(|e| e.to_string())?;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": content
                }],
                "isError": false
            }))
        }

        "save_wiki_page" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let path = args.get("path").and_then(|s| s.as_str())
                .ok_or("Missing path")?;
            let content = args.get("content").and_then(|s| s.as_str())
                .ok_or("Missing content")?;

            // Call the wiki update_page handler logic
            let wiki_repo = WikiRepository::new(pool.clone());
            let result = wiki_handlers::update_page_inner(
                pool, &wiki_repo, project_id, path, content,
            ).await;

            match result {
                Ok(()) => Ok(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Wiki page '{}' saved successfully.", path)
                    }],
                    "isError": false
                })),
                Err(e) => Err(e),
            }
        }

        "search_wiki" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let query = args.get("query").and_then(|s| s.as_str())
                .ok_or("Missing query")?;
            let top_k = args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(10) as usize;

            let wiki_repo = WikiRepository::new(pool.clone());
            let results = wiki_repo.search_pages(project_id, query, top_k)
                .await.map_err(|e| e.to_string())?;

            if results.is_empty() {
                return Ok(serde_json::json!({
                    "content": [{ "type": "text", "text": "No matching wiki pages found." }],
                    "isError": false
                }));
            }

            let lines: Vec<String> = results.iter().map(|r| {
                let mut line = format!("- {} ({})", r.title, r.path);
                if !r.snippet.is_empty() {
                    line.push_str(&format!("\n  {}", r.snippet));
                }
                line
            }).collect();

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Wiki search results ({} found):\n{}", results.len(), lines.join("\n"))
                }],
                "isError": false
            }))
        }

        "ask_wiki" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let question = args.get("question").and_then(|s| s.as_str())
                .ok_or("Missing question")?;
            let top_k = args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(5) as usize;
            let model = args.get("model").and_then(|m| m.as_str());

            let result = wiki_handlers::ask_inner(
                shared, project_id, question, top_k, model,
            ).await;

            match result {
                Ok(json) => Ok(serde_json::json!({
                    "content": [{ "type": "text", "text": serde_json::to_string_pretty(&json).unwrap_or_default() }],
                    "isError": false
                })),
                Err(e) => Err(e),
            }
        }

        "get_wiki_tags" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(15) as usize;

            let wiki_repo = WikiRepository::new(pool.clone());
            let tags = wiki_repo.get_tags(project_id, limit).await.map_err(|e| e.to_string())?;

            if tags.is_empty() {
                return Ok(serde_json::json!({
                    "content": [{ "type": "text", "text": "No tags found. Tags are auto-extracted from page frontmatter." }],
                    "isError": false
                }));
            }

            let lines: Vec<String> = tags.iter().map(|t| {
                format!("- {} ({})", t.word, t.count)
            }).collect();

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Wiki tags:\n{}", lines.join("\n"))
                }],
                "isError": false
            }))
        }

        "get_wiki_graph" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;

            let wiki_repo = WikiRepository::new(pool.clone());
            let graph = wiki_repo.get_graph(project_id).await.map_err(|e| e.to_string())?;

            let lines: Vec<String> = graph.nodes.iter().map(|n| {
                format!("- {} ({}){}", n.label, n.node_type,
                    n.path.as_deref().map(|p| format!(" [{}]", p)).unwrap_or_default())
            }).collect();

            let edge_lines: Vec<String> = graph.edges.iter().map(|e| {
                format!("  {} --{}--> {}", e.source, e.edge_type, e.target)
            }).collect();

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Graph: {} nodes, {} edges\n\nNodes:\n{}\n\nEdges:\n{}",
                        graph.nodes.len(), graph.edges.len(),
                        lines.join("\n"), edge_lines.join("\n"))
                }],
                "isError": false
            }))
        }

        "list_wiki_sources" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;

            let wiki_repo = WikiRepository::new(pool.clone());
            let sources = wiki_repo.list_sources(project_id).await.map_err(|e| e.to_string())?;

            if sources.is_empty() {
                return Ok(serde_json::json!({
                    "content": [{ "type": "text", "text": "No wiki sources yet." }],
                    "isError": false
                }));
            }

            let lines: Vec<String> = sources.iter().map(|s| {
                format!("- {} | ID: {} | Type: {} | Status: {} | Pages: {}",
                    s.filename, s.id, s.source_type, s.status, s.page_count)
            }).collect();

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Wiki sources ({} total):\n{}", sources.len(), lines.join("\n"))
                }],
                "isError": false
            }))
        }

        "ingest_wiki_source" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let source_id = args.get("source_id").and_then(|s| s.as_str())
                .ok_or("Missing source_id")?;

            let task_id = wiki_ingest::start_ingest_source(
                &shared.app, pool, project_id, source_id,
            ).await.map_err(|e| e.to_string())?;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Wiki ingest started. Task ID: {}. Query the background task to monitor progress.",
                        task_id
                    )
                }],
                "isError": false
            }))
        }

        // ── Wiki tools: Project lifecycle ───────────────────────────
        "create_wiki_project" => {
            let name = args.get("name").and_then(|s| s.as_str())
                .ok_or("Missing name")?;

            let input = crate::services::wiki::models::CreateProjectInput {
                name: name.to_string(),
                description: args.get("description").and_then(|d| d.as_str()).map(|s| s.to_string()),
                ingest_model: args.get("ingest_model").and_then(|m| m.as_str()).map(|s| s.to_string()),
                chat_model: args.get("chat_model").and_then(|m| m.as_str()).map(|s| s.to_string()),
                ingest_channel_id: args.get("ingest_channel_id").and_then(|c| c.as_str()).map(|s| s.to_string()),
                chat_channel_id: args.get("chat_channel_id").and_then(|c| c.as_str()).map(|s| s.to_string()),
                schema_text: args.get("schema_text").and_then(|s| s.as_str()).map(|s| s.to_string()),
            };

            let project_id = wiki_project::new_uuid();
            let schema = input.schema_text.clone().unwrap_or_else(|| {
                crate::services::wiki::repository::DEFAULT_SCHEMA.to_string()
            });

            // Create directory structure
            let dir = wiki_project::init_project_dir(&project_id, &schema).await
                .map_err(|e| e.to_string())?;
            let wiki_dir = dir.to_string_lossy().to_string();

            let wiki_repo = WikiRepository::new(pool.clone());
            let project = match wiki_repo.create_project_with_id(&project_id, &input, &wiki_dir).await {
                Ok(project) => project,
                Err(error) => {
                    if let Err(cleanup_error) = wiki_project::remove_project_dir(&project_id).await {
                        return Err(format!("{}; failed to clean up project directory: {}", error, cleanup_error));
                    }
                    return Err(error);
                }
            };

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Wiki project created successfully.\nID: {}\nName: {}\nDescription: {}\nIngest model: {}\nChat model: {}",
                        project.id,
                        project.name,
                        project.description.as_deref().unwrap_or("N/A"),
                        project.ingest_model.as_deref().unwrap_or("default"),
                        project.chat_model.as_deref().unwrap_or("default")
                    )
                }],
                "isError": false
            }))
        }

        "delete_wiki_project" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;

            let wiki_repo = WikiRepository::new(pool.clone());
            let project = wiki_repo.get_project(project_id).await
                .map_err(|e| e.to_string())?;

            let staged = wiki_project::stage_project_dir_removal(project_id).await
                .map_err(|e| e.to_string())?;
            if let Err(error) = wiki_repo.delete_project(project_id).await {
                if let Some(ref removal) = staged {
                    wiki_project::restore_staged_removal(removal).await
                        .map_err(|restore_error| format!("{}; failed to restore project directory: {}", error, restore_error))?;
                }
                return Err(error);
            }
            if let Some(removal) = staged {
                wiki_project::finalize_staged_removal(removal).await
                    .map_err(|e| e.to_string())?;
            }

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Wiki project '{}' ({}) has been permanently deleted.", project.name, project_id)
                }],
                "isError": false
            }))
        }

        // ── Wiki tools: Page deletion ───────────────────────────────
        "delete_wiki_page" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let path = args.get("path").and_then(|s| s.as_str())
                .ok_or("Missing path")?;

            let wiki_repo = WikiRepository::new(pool.clone());
            let staged = wiki_project::stage_page_file_removal(project_id, path).await
                .map_err(|e| e.to_string())?;
            if let Err(error) = wiki_repo.delete_page(project_id, path).await {
                if let Some(ref removal) = staged {
                    wiki_project::restore_staged_removal(removal).await
                        .map_err(|restore_error| format!("{}; failed to restore page file: {}", error, restore_error))?;
                }
                return Err(error);
            }
            if let Some(removal) = staged {
                wiki_project::finalize_staged_removal(removal).await
                    .map_err(|e| e.to_string())?;
            }

            // Rebuild graph edges after page deletion
            let _ = crate::services::wiki::ingest::rebuild_graph_edges(&pool, project_id).await;

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Wiki page '{}' has been deleted.", path)
                }],
                "isError": false
            }))
        }

        // ── Wiki tools: Source management ──────────────────────────
        "add_wiki_source" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let filename = args.get("filename").and_then(|s| s.as_str())
                .ok_or("Missing filename")?;
            let source_type = args.get("source_type").and_then(|s| s.as_str())
                .ok_or("Missing source_type")?;

            let content = args.get("content").and_then(|c| c.as_str());
            let file_path = args.get("file_path").and_then(|f| f.as_str()).map(|s| s.to_string());
            let source_url = args.get("source_url").and_then(|u| u.as_str()).map(|s| s.to_string());

            // Compute hash and size if content provided
            let (content_hash, file_size) = if let Some(ref content) = content {
                use sha2::Digest;
                let mut hasher = sha2::Sha256::new();
                hasher.update(content.as_bytes());
                let hash = format!("{:x}", hasher.finalize());
                (Some(hash), content.len() as i64)
            } else {
                (None, 0i64)
            };

            // Write content to disk if provided and persist the managed path.
            let written_path = if let Some(ref content) = content {
                Some(wiki_project::write_source_file(project_id, filename, content.as_bytes()).await
                    .map_err(|e| e.to_string())?)
            } else {
                None
            };

            let mut input = crate::services::wiki::models::AddSourceInput {
                source_type: source_type.to_string(),
                filename: filename.to_string(),
                file_path,
                source_url,
                content: content.map(|s| s.to_string()),
            };
            if let Some(path) = &written_path {
                input.file_path = Some(path.to_string_lossy().to_string());
            }

            let wiki_repo = WikiRepository::new(pool.clone());
            let source = match wiki_repo.add_source(project_id, &input, content_hash.as_deref(), file_size).await {
                Ok(source) => source,
                Err(error) => {
                    if let Some(path) = written_path {
                        if let Err(cleanup_error) = tokio::fs::remove_file(&path).await {
                            return Err(format!("{}; failed to clean up source file: {}", error, cleanup_error));
                        }
                    }
                    return Err(error);
                }
            };

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Wiki source added successfully.\nID: {}\nFilename: {}\nType: {}\nStatus: pending\n\nUse ingest_wiki_source to generate structured pages from this source.",
                        source.id, source.filename, source.source_type
                    )
                }],
                "isError": false
            }))
        }

        "delete_wiki_source" => {
            let project_id = args.get("project_id").and_then(|s| s.as_str())
                .ok_or("Missing project_id")?;
            let source_id = args.get("source_id").and_then(|s| s.as_str())
                .ok_or("Missing source_id")?;

            let wiki_repo = WikiRepository::new(pool.clone());

            // Get source info before deletion for the response message
            let sources = wiki_repo.list_sources(project_id).await
                .map_err(|e| e.to_string())?;
            let source = sources.iter().find(|s| s.id == source_id)
                .ok_or_else(|| format!("Source {} not found in project {}", source_id, project_id))?;

            let filename = source.filename.clone();
            let staged = wiki_project::stage_source_file_removal(
                project_id,
                &source.filename,
                source.file_path.as_deref(),
            ).await.map_err(|e| e.to_string())?;
            if let Err(error) = wiki_repo.delete_source(source_id).await {
                if let Some(ref removal) = staged {
                    wiki_project::restore_staged_removal(removal).await
                        .map_err(|restore_error| format!("{}; failed to restore source file: {}", error, restore_error))?;
                }
                return Err(error);
            }
            if let Some(removal) = staged {
                wiki_project::finalize_staged_removal(removal).await
                    .map_err(|e| e.to_string())?;
            }

            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Wiki source '{}' ({}) has been deleted.", filename, source_id)
                }],
                "isError": false
            }))
        }

        _ => Err(format!("Unknown tool: {}", tool_name)),
    }
}
