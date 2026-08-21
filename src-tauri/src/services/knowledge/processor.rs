use super::code_parser;
use super::embedder;
use super::parser;
use super::splitter;
use super::repository::{KbRepository, ChunkInsert, KB_CONFIG_SUPERSEDED};
use super::retriever;
use crate::db::models::now_iso;
use crate::db::repository::Repository;
use crate::services::tasks::{
    emit_task_event,
    models::TASK_CANCELLED,
    repository::TaskRepository,
};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

/// Default embedding model
const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";
pub const DOCUMENT_TASK_ALREADY_RUNNING: &str = "KB_DOCUMENT_TASK_ALREADY_RUNNING";

async fn ensure_task_tree_not_cancelled(
    tasks: &TaskRepository,
    task_id: &str,
) -> Result<(), String> {
    let task = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    if let Some(parent_task_id) = task.parent_task_id.as_deref() {
        if tasks.ensure_not_cancelled(parent_task_id).await.is_err() {
            let _ = tasks.mark_cancelled(task_id).await;
            return Err(TASK_CANCELLED.to_string());
        }
    }
    tasks.ensure_not_cancelled(task_id).await
}

/// Emit progress event to frontend
async fn emit_progress(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
    doc_id: &str,
    kb_id: &str,
    filename: &str,
    stage: &str,
    progress: u8,
    detail: &str,
) {
    let tasks = TaskRepository::new(pool.clone());
    if let Ok(task) = tasks.get(task_id).await {
        let done_items = if progress >= 100 { task.total_items } else { task.done_items };
        if let Err(error) = tasks
            .update_progress(
                task_id,
                stage,
                i64::from(progress),
                done_items,
                task.total_items,
            )
            .await
        {
            tracing::warn!(%error, task_id, "failed to persist document task progress");
        }
        if let Ok(updated) = tasks.get(task_id).await {
            emit_task_event(app, &updated, Some(detail));
        }
    }
    let _ = app.emit("kb-document-progress", serde_json::json!({
        "task_id": task_id,
        "doc_id": doc_id,
        "kb_id": kb_id,
        "filename": filename,
        "stage": stage,
        "progress": progress,
        "detail": detail,
    }));
}

/// Process an uploaded document: parse → split → embed → store
pub async fn process_document(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    filename: &str,
    content: &[u8],
    embedding_model: Option<&str>,
) -> Result<String, String> {
    process_document_with_parent(
        pool,
        app,
        kb_id,
        doc_id,
        filename,
        content,
        embedding_model,
        None,
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn process_document_with_parent(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    filename: &str,
    content: &[u8],
    embedding_model: Option<&str>,
    parent_task_id: Option<&str>,
    retryable: bool,
) -> Result<String, String> {
    let repo = KbRepository::new(pool.clone());
    let task = repo
        .create_task_if_idle_with_options(
            kb_id,
            Some(doc_id),
            "process_document",
            1,
            parent_task_id,
            retryable,
        )
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| DOCUMENT_TASK_ALREADY_RUNNING.to_string())?;
    let task_id = task.id.clone();
    let rebuild_index = parent_task_id.is_none();
    run_document_task(
        pool,
        app,
        kb_id,
        doc_id,
        filename,
        content,
        embedding_model,
        &task_id,
        "failed",
        rebuild_index,
    )
    .await?;
    Ok(task_id)
}

#[allow(clippy::too_many_arguments)]
pub async fn start_document_processing(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    filename: String,
    content: Vec<u8>,
    embedding_model: Option<String>,
    parent_task_id: Option<String>,
    retryable: bool,
) -> Result<String, String> {
    let task = KbRepository::new(pool.clone())
        .create_task_if_idle_with_options(
            kb_id,
            Some(doc_id),
            "process_document",
            1,
            parent_task_id.as_deref(),
            retryable,
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| DOCUMENT_TASK_ALREADY_RUNNING.to_string())?;
    let task_id = task.id.clone();
    let rebuild_index = parent_task_id.is_none();
    let pool = pool.clone();
    let app = app.clone();
    let kb_id = kb_id.to_string();
    let doc_id = doc_id.to_string();
    tokio::spawn(async move {
        if let Err(error) = run_document_task(
            &pool,
            &app,
            &kb_id,
            &doc_id,
            &filename,
            &content,
            embedding_model.as_deref(),
            &task.id,
            "failed",
            rebuild_index,
        )
        .await
        {
            tracing::error!(%error, task_id = %task.id, "knowledge document processing failed");
        }
    });
    Ok(task_id)
}

#[allow(clippy::too_many_arguments)]
async fn run_document_task(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    filename: &str,
    content: &[u8],
    embedding_model: Option<&str>,
    task_id: &str,
    failure_status: &str,
    rebuild_index: bool,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());

    // Update status to processing
    if let Err(error) = repo.update_document_status(doc_id, "processing", None).await {
        let message = error.to_string();
        let _ = repo.complete_task(task_id, Some(&message)).await;
        return Err(message);
    }

    ensure_task_tree_not_cancelled(&TaskRepository::new(pool.clone()), task_id).await?;
    emit_progress(pool, app, task_id, doc_id, kb_id, filename, "processing", 0, "开始处理").await;

    let result = process_document_inner(
        pool,
        app,
        kb_id,
        doc_id,
        filename,
        content,
        embedding_model,
        task_id,
        rebuild_index,
    ).await;

    if let Err(ref e) = result {
        if e == TASK_CANCELLED || e == KB_CONFIG_SUPERSEDED {
            let cancelled_status = if e == KB_CONFIG_SUPERSEDED {
                "stale"
            } else if failure_status == "ready" {
                "ready"
            } else {
                "cancelled"
            };
            let _ = repo.update_document_status(doc_id, cancelled_status, None).await;
            let _ = app.emit("kb-document-progress", serde_json::json!({
                "task_id": task_id,
                "doc_id": doc_id,
                "kb_id": kb_id,
                "filename": filename,
                "stage": "cancelled",
                "progress": 0,
                "detail": "已取消",
            }));
        } else {
            let err_msg = format!("文档「{}」处理失败: {}", filename, e);
            let _ = repo.update_document_status(doc_id, failure_status, Some(&err_msg)).await;
            let _ = repo.complete_task(task_id, Some(&err_msg)).await;
            let _ = app.emit("kb-document-error", serde_json::json!({
                "task_id": task_id,
                "doc_id": doc_id,
                "kb_id": kb_id,
                "filename": filename,
                "error": "文档处理失败",
            }));
        }
    } else {
        emit_progress(pool, app, task_id, doc_id, kb_id, filename, "completed", 100, "处理完成").await;
        let _ = repo.complete_task(task_id, None).await;
    }
    if let Ok(task) = TaskRepository::new(pool.clone()).get(task_id).await {
        emit_task_event(app, &task, task.error_message.as_deref());
    }

    result
}

async fn process_document_inner(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
    filename: &str,
    content: &[u8],
    embedding_model: Option<&str>,
    task_id: &str,
    rebuild_index: bool,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());

    // 1. Parse file
    let tasks = TaskRepository::new(pool.clone());
    ensure_task_tree_not_cancelled(&tasks, task_id).await?;
    emit_progress(pool, app, task_id, doc_id, kb_id, filename, "parsing", 5, "解析文件").await;
    let parsed = parser::parse_file(filename, content)?;

    let (text, file_type_label): (String, String) = match &parsed {
        parser::ParsedContent::PlainText(t) => (t.clone(), "text".to_string()),
        parser::ParsedContent::Markdown { text } => (text.clone(), "markdown".to_string()),
        parser::ParsedContent::Code { text, language } => (text.clone(), language.clone()),
        parser::ParsedContent::Structured(t) => (t.clone(), "structured".to_string()),
    };

    // 2. Split into chunks — use KB-level config if available
    emit_progress(pool, app, task_id, doc_id, kb_id, filename, "splitting", 15, "文本分块").await;
    let kb = repo.get_kb(kb_id).await.map_err(|e| e.to_string())?;
    let config = splitter::SplitConfig {
        chunk_size: if kb.chunk_size > 0 { kb.chunk_size as usize } else { 512 },
        chunk_overlap: if kb.chunk_overlap > 0 { kb.chunk_overlap as usize } else { 64 },
    };
    let base_metadata = splitter::ChunkMetadata {
        file_path: Some(filename.to_string()),
        ..Default::default()
    };

    // 符号感知分块：代码文件且语言受支持时，按 AST 符号边界切分
    let chunks = match &parsed {
        parser::ParsedContent::Code { text, language } => {
            if code_parser::is_supported_language(language) {
                let symbols = code_parser::extract_symbols(filename, text);
                emit_progress(
                    pool, app, task_id, doc_id, kb_id, filename, "splitting", 18,
                    &format!("AST 解析：提取到 {} 个符号", symbols.len()),
                ).await;
                splitter::split_code_by_symbols(text, &symbols, &config, &base_metadata)
            } else {
                splitter::split(text, &file_type_label, &config, &base_metadata)
            }
        }
        _ => splitter::split(&text, &file_type_label, &config, &base_metadata),
    };

    if chunks.is_empty() {
        repo.replace_document_chunks_for_config(kb_id, doc_id, &[], 0, kb.config_revision)
            .await
            .map_err(|e| e.to_string())?;
        if rebuild_index {
            retriever::schedule_index_build(pool, kb_id, app).await?;
        }
        return Ok(());
    }

    let total_tokens: i64 = chunks.iter().map(|c| c.token_count as i64).sum();

    // 3. Embed chunks in batches
    let emb_model = embedding_model.unwrap_or(DEFAULT_EMBEDDING_MODEL);
    let main_repo = Repository::new(pool.clone());

    // Detect expected embedding dimension from KB config
    let expected_dim = if kb.embedding_dim > 0 { Some(kb.embedding_dim as usize) } else { None };

    let batch_size = 32;
    let total_batches = ((chunks.len() as f64) / batch_size as f64).ceil() as usize;
    let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(chunks.len());
    let mut observed_dim = expected_dim;
    let mut batch_done = 0usize;

    for batch in chunks.chunks(batch_size) {
        ensure_task_tree_not_cancelled(&tasks, task_id).await?;
        let texts: Vec<String> = batch.iter().map(|c| c.content.clone()).collect();
        let embeddings = embedder::embed_with_channel(
            &texts,
            emb_model,
            &main_repo,
            kb.embedding_channel_id.as_deref(),
        ).await?;

        if embeddings.len() != batch.len() {
            return Err(format!(
                "Embedding response count mismatch: expected {}, got {}",
                batch.len(),
                embeddings.len()
            ));
        }
        for (index, embedding) in embeddings.iter().enumerate() {
            if embedding.is_empty() {
                return Err(format!(
                    "Embedding response is empty for chunk {} in batch {}",
                    index, batch_done
                ));
            }
            match observed_dim {
                Some(dim) if embedding.len() != dim => {
                    return Err(format!(
                        "Embedding dimension mismatch: expected {}, got {} for chunk {} in batch {}",
                        dim,
                        embedding.len(),
                        index,
                        batch_done
                    ));
                }
                None => observed_dim = Some(embedding.len()),
                _ => {}
            }
        }

        all_embeddings.extend(embeddings);
        batch_done += 1;
        // Embedding progress: 20% ~ 80%
        let pct = 20 + ((batch_done as f64 / total_batches as f64) * 60.0) as u8;
        emit_progress(
            pool, app, task_id, doc_id, kb_id, filename, "embedding", pct,
            &format!("向量化 {}/{}", batch_done, total_batches),
        ).await;
    }

    // Auto-detect and update KB embedding dimension if not set
    if expected_dim.is_none() {
        let detected_dim = observed_dim
            .ok_or_else(|| "Embedding response did not contain a valid dimension".to_string())?
            as i64;
        tracing::info!("Auto-detected embedding dim {} for KB {}", detected_dim, kb_id);
        if let Err(error) = repo.update_kb_embedding_dim(kb_id, detected_dim).await {
            tracing::warn!(%error, knowledge_base_id = %kb_id, "failed to persist embedding dimension");
        }
    }

    // 4. Prepare all chunks before replacing the previous document snapshot.
    let chunks_total = chunks.len();
    let mut inserts = Vec::with_capacity(chunks_total);
    for (i, chunk) in chunks.iter().enumerate() {
        // Storing progress: 80% ~ 95%
        if i % 10 == 0 || i == chunks_total - 1 {
            let pct = 80 + ((i as f64 + 1.0) / chunks_total as f64 * 15.0) as u8;
            emit_progress(
                pool, app, task_id, doc_id, kb_id, filename, "storing", pct,
                &format!("存储切片 {}/{}", i + 1, chunks_total),
            ).await;
        }
        let embedding_bytes = retriever::encode_embedding(&all_embeddings[i]);
        inserts.push(ChunkInsert {
            id: uuid::Uuid::new_v4().to_string(),
            doc_id: doc_id.to_string(),
            kb_id: kb_id.to_string(),
            chunk_index: i as i64,
            content: chunk.content.clone(),
            token_count: chunk.token_count as i64,
            embedding: embedding_bytes,
            embedding_dim: all_embeddings[i].len() as i64,
            metadata: serde_json::to_string(&chunk.metadata).unwrap_or_else(|_| "{}".to_string()),
            created_at: now_iso(),
        });
    }

    // 5. Atomically swap chunks, document status, counts and index freshness.
    ensure_task_tree_not_cancelled(&tasks, task_id).await?;
    emit_progress(pool, app, task_id, doc_id, kb_id, filename, "finalizing", 98, "更新统计").await;
    repo.replace_document_chunks_for_config(
        kb_id,
        doc_id,
        &inserts,
        total_tokens,
        kb.config_revision,
    )
        .await
        .map_err(|e| e.to_string())?;

    if rebuild_index {
        ensure_task_tree_not_cancelled(&tasks, task_id).await?;
        emit_progress(
            pool,
            app,
            task_id,
            doc_id,
            kb_id,
            filename,
            "indexing",
            99,
            "更新向量索引",
        )
        .await;
        if let Err(error) = retriever::schedule_index_build(pool, kb_id, app).await {
            tracing::warn!(%error, knowledge_base_id = %kb_id, "failed to schedule HNSW rebuild");
        }
    }

    Ok(())
}

/// Reindex a document while preserving the previous ready snapshot on failure.
pub async fn reindex_document(
    pool: &SqlitePool,
    app: &AppHandle,
    doc_id: &str,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let doc = repo.get_document(doc_id).await.map_err(|e| e.to_string())?;
    let task = repo
        .create_task_if_idle(&doc.kb_id, Some(doc_id), "reindex_document", 1)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| DOCUMENT_TASK_ALREADY_RUNNING.to_string())?;
    run_reindex_task(pool, app, doc, task.id, true, true, None).await
}

pub async fn start_reindex_document(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    doc_id: &str,
) -> Result<String, String> {
    let repo = KbRepository::new(pool.clone());
    let doc = repo.get_document(doc_id).await.map_err(|e| e.to_string())?;
    if doc.kb_id != kb_id {
        return Err("DOCUMENT_NOT_FOUND".to_string());
    }
    let task = repo
        .create_task_if_idle(kb_id, Some(doc_id), "reindex_document", 1)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| DOCUMENT_TASK_ALREADY_RUNNING.to_string())?;
    let task_id = task.id.clone();
    let pool = pool.clone();
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = run_reindex_task(&pool, &app, doc, task.id, true, true, None).await {
            tracing::error!(%error, "knowledge document reindex failed");
        }
    });
    Ok(task_id)
}

/// Reprocess a document as part of a persisted knowledge-base configuration
/// revision. The parent owns index construction, so this child only swaps the
/// document snapshot and never starts a competing index build.
pub async fn reprocess_document_with_parent(
    pool: &SqlitePool,
    app: &AppHandle,
    doc: super::models::KbDocument,
    parent_task_id: &str,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let task = repo
        .create_task_if_idle_with_options(
            &doc.kb_id,
            Some(&doc.id),
            "reindex_document",
            1,
            Some(parent_task_id),
            false,
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| DOCUMENT_TASK_ALREADY_RUNNING.to_string())?;
    run_reindex_task(
        pool,
        app,
        doc,
        task.id,
        false,
        false,
        Some("stale"),
    )
    .await
}

pub async fn start_existing_document_task(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
) -> Result<(), String> {
    let tasks = TaskRepository::new(pool.clone());
    let task = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    if task.domain != "knowledge"
        || !matches!(task.task_type.as_str(), "process_document" | "reindex_document")
        || task.resource_type != "knowledge_base"
        || task.status != "pending"
    {
        return Err("后台任务不是可执行的知识库文档任务".to_string());
    }
    let payload: serde_json::Value = serde_json::from_str(&task.payload_json)
        .map_err(|error| format!("知识库文档任务参数损坏: {}", error))?;
    if payload.get("payload_version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("知识库文档任务参数版本不受支持".to_string());
    }
    if payload.get("operation").and_then(serde_json::Value::as_str)
        != Some(task.task_type.as_str())
    {
        return Err("知识库文档任务操作类型不匹配".to_string());
    }
    if payload.get("kb_id").and_then(serde_json::Value::as_str)
        != Some(task.resource_id.as_str())
    {
        return Err("知识库文档任务资源参数不匹配".to_string());
    }
    let doc_id = task
        .subject_id
        .as_deref()
        .ok_or_else(|| "知识库文档任务缺少文档标识".to_string())?;
    if payload.get("doc_id").and_then(serde_json::Value::as_str) != Some(doc_id) {
        return Err("知识库文档任务文档参数不匹配".to_string());
    }
    let doc = KbRepository::new(pool.clone())
        .get_document(doc_id)
        .await
        .map_err(|error| error.to_string())?;
    if doc.kb_id != task.resource_id {
        return Err("知识库文档任务资源不匹配".to_string());
    }
    if !tasks.claim(task_id, "processing").await.map_err(|error| error.to_string())? {
        return Err("知识库文档任务已经开始或结束".to_string());
    }
    let pool = pool.clone();
    let app = app.clone();
    let preserve_existing_chunks = task.task_type == "reindex_document";
    let rebuild_index = task.parent_task_id.is_none();
    let task_id = task.id;
    tokio::spawn(async move {
        if let Err(error) = run_reindex_task(
            &pool,
            &app,
            doc,
            task_id,
            preserve_existing_chunks,
            rebuild_index,
            None,
        )
        .await
        {
            tracing::error!(%error, "retried knowledge document task failed");
        }
    });
    Ok(())
}

async fn run_reindex_task(
    pool: &SqlitePool,
    app: &AppHandle,
    doc: super::models::KbDocument,
    task_id: String,
    preserve_existing_chunks: bool,
    rebuild_index: bool,
    failure_status_override: Option<&str>,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let failure_status = failure_status_override.unwrap_or(if preserve_existing_chunks && doc.chunk_count > 0 {
        "ready"
    } else {
        "failed"
    });

    let content = if let Some(path) = &doc.file_path {
        match tokio::fs::read(path).await {
            Ok(content) => content,
            Err(error) => {
                let message = format!("Failed to read file: {}", error);
                let _ = repo.complete_task(&task_id, Some(&message)).await;
                let _ = repo
                    .update_document_status(&doc.id, failure_status, Some(&message))
                    .await;
                return Err(message);
            }
        }
    } else {
        let message = "No file path to reindex".to_string();
        let _ = repo.complete_task(&task_id, Some(&message)).await;
        let _ = repo
            .update_document_status(&doc.id, failure_status, Some(&message))
            .await;
        return Err(message);
    };

    let kb = match repo.get_kb(&doc.kb_id).await {
        Ok(kb) => kb,
        Err(error) => {
            let message = error.to_string();
            let _ = repo.complete_task(&task_id, Some(&message)).await;
            let _ = repo
                .update_document_status(&doc.id, failure_status, Some(&message))
                .await;
            return Err(message);
        }
    };
    run_document_task(
        pool,
        app,
        &doc.kb_id,
        &doc.id,
        &doc.filename,
        &content,
        kb.embedding_model.as_deref(),
        &task_id,
        failure_status,
        rebuild_index,
    )
    .await
}
