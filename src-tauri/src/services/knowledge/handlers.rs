use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Json, IntoResponse, Response},
};
use serde::Deserialize;
use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::db::repository::Repository;
use tauri::Manager;
use sha2::Digest;
use super::models::*;
use super::repository::KbRepository;
use super::processor;
use super::rag;
use super::embedder;
use super::retriever;

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

// ─── Knowledge Base CRUD ──────────────────────────────────────────

pub async fn list_knowledge_bases(
    State(shared): State<SharedState>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_all_kbs().await {
        Ok(kbs) => Json(serde_json::json!({ "data": kbs })).into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_BASE_LIST_FAILED",
            "读取知识库失败",
            error,
        ).into_response(),
    }
}

pub async fn create_knowledge_base(
    State(shared): State<SharedState>,
    Json(input): Json<CreateKbInput>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.create_kb(&input).await {
        Ok(kb) => (StatusCode::CREATED, Json(kb)).into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_BASE_CREATE_FAILED",
            "创建知识库失败",
            error,
        ).into_response(),
    }
}

pub async fn get_knowledge_base(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_kb(&id).await {
        Ok(kb) => Json(kb).into_response(),
        Err(sqlx::Error::RowNotFound) => HttpError::not_found(
            "KNOWLEDGE_BASE_NOT_FOUND",
            "知识库不存在",
        ).into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_BASE_READ_FAILED",
            "读取知识库失败",
            error,
        ).into_response(),
    }
}

pub async fn update_knowledge_base(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateKbInput>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.update_kb(&id, &input).await {
        Ok(kb) => Json(kb).into_response(),
        Err(sqlx::Error::RowNotFound) => HttpError::not_found(
            "KNOWLEDGE_BASE_NOT_FOUND",
            "知识库不存在",
        ).into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_BASE_UPDATE_FAILED",
            "更新知识库失败",
            error,
        ).into_response(),
    }
}

pub async fn delete_knowledge_base(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.delete_kb(&id).await {
        Ok(_) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_BASE_DELETE_FAILED",
            "删除知识库失败",
            error,
        ).into_response(),
    }
}

// ─── Document Management ──────────────────────────────────────────

pub async fn list_documents(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_documents(&kb_id).await {
        Ok(docs) => Json(serde_json::json!({ "data": docs })).into_response(),
        Err(error) => HttpError::internal(
            "DOCUMENT_LIST_FAILED",
            "读取文档列表失败",
            error,
        ).into_response(),
    }
}

pub async fn upload_document(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
    Json(input): Json<UploadDocInput>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());

    let content = match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &input.content) {
        Ok(c) => c,
        Err(_) => return HttpError::bad_request(
            "DOCUMENT_CONTENT_INVALID",
            "文档内容不是有效的 Base64 数据",
        ).into_response(),
    };

    let hash = sha2::Sha256::digest(&content);
    let hash_hex = hex::encode(hash);

    match repo.find_document_by_hash(&kb_id, &hash_hex).await {
        Ok(Some(_)) => return HttpError::conflict(
            "DOCUMENT_DUPLICATE",
            "相同内容的文档已存在",
        ).into_response(),
        Ok(None) => {}
        Err(error) => return HttpError::internal(
            "DOCUMENT_DUPLICATE_CHECK_FAILED",
            "检查重复文档失败",
            error,
        ).into_response(),
    }

    if let Err(error) = super::safe_path_component(&input.filename, "filename") {
        return HttpError::bad_request("DOCUMENT_FILENAME_INVALID", error).into_response();
    }

    let file_type = super::parser::get_file_type(&input.filename);
    let file_size = content.len() as i64;

    let app_data_dir = shared.app.path().app_data_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let kb_dir = app_data_dir.join("kb_files").join(&kb_id);
    if let Err(error) = std::fs::create_dir_all(&kb_dir) {
        return HttpError::internal(
            "DOCUMENT_DIRECTORY_CREATE_FAILED",
            "创建文档目录失败",
            error,
        ).into_response();
    }
    let doc_id = uuid::Uuid::new_v4().to_string();
    let file_path = kb_dir.join(format!("{}_{}", &doc_id, &input.filename));
    if let Err(error) = std::fs::write(&file_path, &content) {
        return HttpError::internal(
            "DOCUMENT_FILE_SAVE_FAILED",
            "保存文档失败",
            error,
        ).into_response();
    }
    let file_path_str = file_path.to_string_lossy().to_string();

    let doc = match repo.create_document(
        &kb_id,
        &input.filename,
        Some(&file_path_str),
        &file_type,
        file_size,
        &hash_hex,
    ).await {
        Ok(d) => d,
        Err(e) => {
            if let Err(remove_error) = std::fs::remove_file(&file_path) {
                tracing::warn!(%remove_error, path = %file_path.display(), "failed to remove document after DB insert error");
            }
            return HttpError::internal(
                "DOCUMENT_CREATE_FAILED",
                "创建文档记录失败",
                e,
            ).into_response();
        }
    };

    let kb = match repo.get_kb(&kb_id).await {
        Ok(k) => k,
        Err(sqlx::Error::RowNotFound) => return HttpError::not_found(
            "KNOWLEDGE_BASE_NOT_FOUND",
            "知识库不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "KNOWLEDGE_BASE_READ_FAILED",
            "读取知识库失败",
            error,
        ).into_response(),
    };

    let pool = shared.state.db.pool.clone();
    let app = shared.app.clone();
    let doc_id_clone = doc.id.clone();
    let filename_clone = input.filename.clone();
    let emb_model = kb.embedding_model.clone();

    tokio::spawn(async move {
        if let Err(e) = processor::process_document(
            &pool,
            &app,
            &kb_id,
            &doc_id_clone,
            &filename_clone,
            &content,
            emb_model.as_deref(),
        ).await {
            tracing::error!("Document processing failed: {}", e);
        }
    });

    Json(doc).into_response()
}

pub async fn get_document(
    State(shared): State<SharedState>,
    Path((kb_id, doc_id)): Path<(String, String)>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_document(&doc_id).await {
        Ok(doc) if doc.kb_id == kb_id => Json(doc).into_response(),
        Ok(_) | Err(sqlx::Error::RowNotFound) => HttpError::not_found(
            "DOCUMENT_NOT_FOUND",
            "文档不存在",
        ).into_response(),
        Err(error) => HttpError::internal(
            "DOCUMENT_READ_FAILED",
            "读取文档失败",
            error,
        ).into_response(),
    }
}

pub async fn delete_document(
    State(shared): State<SharedState>,
    Path((kb_id, doc_id)): Path<(String, String)>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());

    let doc = match repo.get_document(&doc_id).await {
        Ok(doc) if doc.kb_id == kb_id => doc,
        Ok(_) | Err(sqlx::Error::RowNotFound) => return HttpError::not_found(
            "DOCUMENT_NOT_FOUND",
            "文档不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "DOCUMENT_READ_FAILED",
            "读取文档失败",
            error,
        ).into_response(),
    };

    if doc.source_type == "upload" {
        if let Some(path) = &doc.file_path {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return HttpError::internal(
                    "DOCUMENT_FILE_DELETE_FAILED",
                    "删除文档文件失败",
                    error,
                ).into_response(),
            }
        }
    }

    match repo.delete_document(&doc_id).await {
        Ok(_) => {
            if let Err(error) = repo.update_kb_counts(&kb_id).await {
                return HttpError::internal(
                    "KNOWLEDGE_BASE_COUNT_UPDATE_FAILED",
                    "文档已删除，但更新知识库统计失败",
                    error,
                ).into_response();
            }
            (StatusCode::NO_CONTENT, "").into_response()
        }
        Err(error) => HttpError::internal(
            "DOCUMENT_DELETE_FAILED",
            "删除文档失败",
            error,
        ).into_response(),
    }
}

pub async fn reindex_document(
    State(shared): State<SharedState>,
    Path((kb_id, doc_id)): Path<(String, String)>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_document(&doc_id).await {
        Ok(document) if document.kb_id == kb_id => {}
        Ok(_) | Err(sqlx::Error::RowNotFound) => return HttpError::not_found(
            "DOCUMENT_NOT_FOUND",
            "文档不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "DOCUMENT_READ_FAILED",
            "读取文档失败",
            error,
        ).into_response(),
    }

    match processor::start_reindex_document(
        &shared.state.db.pool,
        &shared.app,
        &kb_id,
        &doc_id,
    )
    .await
    {
        Ok(task_id) => Json(serde_json::json!({
            "message": "Reindex started",
            "task_id": task_id,
        })).into_response(),
        Err(error) if error == processor::DOCUMENT_TASK_ALREADY_RUNNING => HttpError::conflict(
            "DOCUMENT_REINDEX_ALREADY_RUNNING",
            "该文档正在重新处理",
        ).into_response(),
        Err(error) => HttpError::internal(
            "DOCUMENT_REINDEX_START_FAILED",
            "启动文档重新处理失败",
            error,
        ).into_response(),
    }
}

// ─── Search ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub kb_id: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize { 5 }

pub async fn search(
    State(shared): State<SharedState>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let repo = Repository::new(shared.state.db.pool.clone());

    let (emb_model, embedding_channel_id) = if let Some(kb_id) = &query.kb_id {
        let kb_repo = KbRepository::new(shared.state.db.pool.clone());
        let kb = match kb_repo.get_kb(kb_id).await {
            Ok(kb) => kb,
            Err(sqlx::Error::RowNotFound) => return HttpError::not_found(
                "KNOWLEDGE_BASE_NOT_FOUND",
                "知识库不存在",
            ).into_response(),
            Err(error) => return HttpError::internal(
                "KNOWLEDGE_BASE_READ_FAILED",
                "读取知识库失败",
                error,
            ).into_response(),
        };
        (
            kb.embedding_model.unwrap_or_else(|| "text-embedding-3-small".to_string()),
            kb.embedding_channel_id,
        )
    } else {
        ("text-embedding-3-small".to_string(), None)
    };

    let embeddings = match embedder::embed_with_channel(
        &[query.q.clone()],
        &emb_model,
        &repo,
        embedding_channel_id.as_deref(),
    ).await {
        Ok(e) => e,
        Err(error) => return HttpError::reported(
            StatusCode::BAD_GATEWAY,
            "EMBEDDING_REQUEST_FAILED",
            "生成查询向量失败",
            true,
            error,
        ).into_response(),
    };

    if embeddings.is_empty() {
        return HttpError::new(
            StatusCode::BAD_GATEWAY,
            "EMBEDDING_RESPONSE_INVALID",
            "向量服务未返回有效结果",
            true,
        ).into_response();
    }

    let query_emb = &embeddings[0];

    let results = if let Some(kb_id) = &query.kb_id {
        retriever::search(&shared.state.db.pool, kb_id, query_emb, query.top_k).await
    } else {
        retriever::search_all(&shared.state.db.pool, query_emb, query.top_k, false).await
    };

    match results {
        Ok(results) => Json(serde_json::json!({ "data": results })).into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_SEARCH_FAILED",
            "搜索知识库失败",
            error,
        ).into_response(),
    }
}

// ─── RAG Ask (with history + token fallback) ──────────────────────

pub async fn ask(
    State(shared): State<SharedState>,
    Json(input): Json<AskInput>,
) -> Response {
    let kb_id = input.kb_id.clone().unwrap_or_default();

    let emb_model = if !kb_id.is_empty() {
        let kb_repo = KbRepository::new(shared.state.db.pool.clone());
        let kb = match kb_repo.get_kb(&kb_id).await {
            Ok(kb) => kb,
            Err(sqlx::Error::RowNotFound) => return HttpError::not_found(
                "KNOWLEDGE_BASE_NOT_FOUND",
                "知识库不存在",
            ).into_response(),
            Err(error) => return HttpError::internal(
                "KNOWLEDGE_BASE_READ_FAILED",
                "读取知识库失败",
                error,
            ).into_response(),
        };
        kb.embedding_model
            .unwrap_or_else(|| "text-embedding-3-small".to_string())
    } else {
        "text-embedding-3-small".to_string()
    };

    // Deep Research mode
    if input.deep_research && !kb_id.is_empty() {
        match rag::deep_research(
            &shared.state.db.pool,
            &kb_id,
            &input.question,
            &emb_model,
            &input.model,
            input.top_k,
            input.max_rounds,
            &shared.app,
        ).await {
            Ok(answer) => Json(answer).into_response(),
            Err(error) => HttpError::internal(
                "DEEP_RESEARCH_FAILED",
                "深度研究失败",
                error,
            ).into_response(),
        }
    } else {
        // Normal RAG with history and configurable search
        let history = input.history.unwrap_or_default();
        let vector_weight = input.vector_weight.unwrap_or(0.7);
        let keyword_weight = input.keyword_weight.unwrap_or(0.3);
        let search_mode = input.search_mode.as_deref().unwrap_or("hybrid");

        match rag::ask_with_config(
            &shared.state.db.pool,
            &kb_id,
            &input.question,
            &emb_model,
            &input.model,
            input.top_k,
            false,
            &history,
            &shared.app,
            vector_weight,
            keyword_weight,
            search_mode,
        ).await {
            Ok(answer) => Json(answer).into_response(),
            Err(error) => HttpError::internal(
                "KNOWLEDGE_ASK_FAILED",
                "知识库问答失败",
                error,
            ).into_response(),
        }
    }
}

// ─── Stats ────────────────────────────────────────────────────────

pub async fn kb_stats(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());

    let kb = match repo.get_kb(&kb_id).await {
        Ok(k) => k,
        Err(sqlx::Error::RowNotFound) => return HttpError::not_found(
            "KNOWLEDGE_BASE_NOT_FOUND",
            "知识库不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "KNOWLEDGE_BASE_READ_FAILED",
            "读取知识库失败",
            error,
        ).into_response(),
    };

    let docs = match repo.get_documents(&kb_id).await {
        Ok(docs) => docs,
        Err(error) => return HttpError::internal(
            "DOCUMENT_LIST_FAILED",
            "读取文档统计失败",
            error,
        ).into_response(),
    };
    let ready_count = docs.iter().filter(|d| d.status == "ready").count();
    let processing_count = docs.iter().filter(|d| d.status == "processing").count();
    let failed_count = docs.iter().filter(|d| d.status == "failed").count();
    let pending_count = docs.iter().filter(|d| d.status == "pending").count();

    let index_meta = match repo.get_index_meta(&kb_id).await {
        Ok(index_meta) => index_meta,
        Err(error) => return HttpError::internal(
            "INDEX_STATUS_READ_FAILED",
            "读取索引状态失败",
            error,
        ).into_response(),
    };

    Json(serde_json::json!({
        "kb": kb,
        "documents": {
            "total": docs.len(),
            "ready": ready_count,
            "processing": processing_count,
            "failed": failed_count,
            "pending": pending_count,
        },
        "index": index_meta,
    })).into_response()
}

// ════════════════════════════════════════════════════════
// New endpoints: Conversation History, Sources, Index, Import
// ════════════════════════════════════════════════════════

// ─── Conversation History ─────────────────────────────────────────

pub async fn list_conversations(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_conversations(&kb_id).await {
        Ok(convs) => Json(serde_json::json!({ "data": convs })).into_response(),
        Err(error) => HttpError::internal(
            "CONVERSATION_LIST_FAILED",
            "读取会话记录失败",
            error,
        ).into_response(),
    }
}

pub async fn clear_conversations(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.clear_conversations(&kb_id).await {
        Ok(_) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(error) => HttpError::internal(
            "CONVERSATION_CLEAR_FAILED",
            "清空会话记录失败",
            error,
        ).into_response(),
    }
}

// ─── Sources ──────────────────────────────────────────────────────

pub async fn list_sources(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_sources(&kb_id).await {
        Ok(sources) => Json(serde_json::json!({ "data": sources })).into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_SOURCE_LIST_FAILED",
            "读取知识来源失败",
            error,
        ).into_response(),
    }
}

pub async fn delete_source(
    State(shared): State<SharedState>,
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.delete_source_with_documents(&kb_id, &source_id).await {
        Ok(_) => {
            if let Err(error) = repo.update_kb_counts(&kb_id).await {
                return HttpError::internal(
                    "KNOWLEDGE_BASE_COUNT_UPDATE_FAILED",
                    "来源已删除，但更新知识库统计失败",
                    error,
                ).into_response();
            }
            (StatusCode::NO_CONTENT, "").into_response()
        }
        Err(sqlx::Error::RowNotFound) => HttpError::not_found(
            "KNOWLEDGE_SOURCE_NOT_FOUND",
            "知识来源不存在",
        ).into_response(),
        Err(error) => HttpError::internal(
            "KNOWLEDGE_SOURCE_DELETE_FAILED",
            "删除知识来源失败",
            error,
        ).into_response(),
    }
}

pub async fn import_source(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
    Json(input): Json<ImportSourceInput>,
) -> Response {
    let pool = shared.state.db.pool.clone();
    let app = shared.app.clone();

    let repo = KbRepository::new(pool.clone());

    // Create source record
    let source = match repo.create_source(
        &kb_id,
        &input.source_type,
        input.repo_url.as_deref().or(input.url.as_deref()),
        input.dir_path.as_deref(),
        input.branch.as_deref(),
    ).await {
        Ok(s) => s,
        Err(error) => return HttpError::internal(
            "KNOWLEDGE_SOURCE_CREATE_FAILED",
            "创建知识来源失败",
            error,
        ).into_response(),
    };

    let source_id = source.id.clone();
    let source_type = input.source_type.clone();

    tokio::spawn(async move {
        let result = if source_type == "git" {
            super::importer::import_git_repo(
                &pool, &app, &kb_id, &source_id, &input,
            ).await
        } else if source_type == "url" {
            super::importer::import_url(
                &pool, &app, &kb_id, &source_id, &input,
            ).await
        } else if source_type == "local_dir" {
            super::importer::import_local_dir(
                &pool, &app, &kb_id, &source_id, &input,
            ).await
        } else {
            Err(format!("Unknown source type: {}", source_type))
        };

        let repo = KbRepository::new(pool.clone());
        match result {
            Ok(count) => {
                if let Err(error) = repo.update_source_status(&source_id, "done", count as i64, None).await {
                    tracing::warn!(%error, source_id = %source_id, "failed to persist knowledge source completion");
                }
            }
            Err(e) => {
                if let Err(error) = repo.update_source_status(&source_id, "error", 0, Some(&e)).await {
                    tracing::warn!(%error, source_id = %source_id, "failed to persist knowledge source failure");
                }
                tracing::error!("Import failed: {}", e);
            }
        }
    });

    Json(source).into_response()
}

// ─── Index Management ─────────────────────────────────────────────

pub async fn get_index_status(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    let repo = KbRepository::new(shared.state.db.pool.clone());
    match repo.get_index_meta(&kb_id).await {
        Ok(meta) => Json(serde_json::json!({ "data": meta })).into_response(),
        Err(error) => HttpError::internal(
            "INDEX_STATUS_READ_FAILED",
            "读取索引状态失败",
            error,
        ).into_response(),
    }
}

pub async fn build_index(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    match retriever::start_index_build(&shared.state.db.pool, &kb_id, &shared.app).await {
        Ok(task_id) => Json(serde_json::json!({
            "message": "Index build started",
            "task_id": task_id,
        })).into_response(),
        Err(error) if error == retriever::INDEX_BUILD_ALREADY_RUNNING => HttpError::conflict(
            "INDEX_BUILD_ALREADY_RUNNING",
            "该知识库的索引正在构建",
        ).into_response(),
        Err(error) => HttpError::internal(
            "INDEX_BUILD_START_FAILED",
            "启动索引构建失败",
            error,
        ).into_response(),
    }
}

pub async fn drop_index(
    State(shared): State<SharedState>,
    Path(kb_id): Path<String>,
) -> Response {
    let pool = shared.state.db.pool.clone();

    match retriever::drop_index(&pool, &kb_id).await {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(e) => {
            tracing::error!("Failed to drop index for KB {}: {}", kb_id, e);
            HttpError::internal(
                "INDEX_DROP_FAILED",
                "删除向量索引失败",
                e,
            ).into_response()
        }
    }
}
