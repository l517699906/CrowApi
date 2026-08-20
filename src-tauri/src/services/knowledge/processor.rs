use super::code_parser;
use super::embedder;
use super::parser;
use super::splitter;
use super::repository::{KbRepository, ChunkInsert};
use super::retriever;
use crate::db::models::now_iso;
use crate::db::repository::Repository;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

/// Default embedding model
const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";
pub const DOCUMENT_TASK_ALREADY_RUNNING: &str = "KB_DOCUMENT_TASK_ALREADY_RUNNING";

/// Emit progress event to frontend
fn emit_progress(
    app: &AppHandle,
    task_id: &str,
    doc_id: &str,
    kb_id: &str,
    filename: &str,
    stage: &str,
    progress: u8,
    detail: &str,
) {
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
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let task = repo
        .create_task(kb_id, Some(doc_id), "process_document", 1)
        .await
        .map_err(|e| e.to_string())?;
    run_document_task(
        pool,
        app,
        kb_id,
        doc_id,
        filename,
        content,
        embedding_model,
        &task.id,
        "failed",
    )
    .await
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
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());

    // Update status to processing
    if let Err(error) = repo.update_document_status(doc_id, "processing", None).await {
        let message = error.to_string();
        let _ = repo.complete_task(task_id, Some(&message)).await;
        return Err(message);
    }

    emit_progress(app, task_id, doc_id, kb_id, filename, "processing", 0, "开始处理");

    let result = process_document_inner(
        pool, app, kb_id, doc_id, filename, content, embedding_model, task_id,
    ).await;

    if let Err(ref e) = result {
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
    } else {
        let _ = repo.update_task_progress(task_id, 1, 100).await;
        let _ = repo.complete_task(task_id, None).await;
        emit_progress(app, task_id, doc_id, kb_id, filename, "done", 100, "处理完成");
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
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());

    // 1. Parse file
    emit_progress(app, task_id, doc_id, kb_id, filename, "parsing", 5, "解析文件");
    let parsed = parser::parse_file(filename, content)?;

    let (text, file_type_label): (String, String) = match &parsed {
        parser::ParsedContent::PlainText(t) => (t.clone(), "text".to_string()),
        parser::ParsedContent::Markdown { text } => (text.clone(), "markdown".to_string()),
        parser::ParsedContent::Code { text, language } => (text.clone(), language.clone()),
        parser::ParsedContent::Structured(t) => (t.clone(), "structured".to_string()),
    };

    // 2. Split into chunks — use KB-level config if available
    emit_progress(app, task_id, doc_id, kb_id, filename, "splitting", 15, "文本分块");
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
                    app, task_id, doc_id, kb_id, filename, "splitting", 18,
                    &format!("AST 解析：提取到 {} 个符号", symbols.len()),
                );
                splitter::split_code_by_symbols(text, &symbols, &config, &base_metadata)
            } else {
                splitter::split(text, &file_type_label, &config, &base_metadata)
            }
        }
        _ => splitter::split(&text, &file_type_label, &config, &base_metadata),
    };

    if chunks.is_empty() {
        repo.replace_document_chunks(kb_id, doc_id, &[], 0)
            .await
            .map_err(|e| e.to_string())?;
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
    let mut batch_done = 0usize;

    for batch in chunks.chunks(batch_size) {
        let texts: Vec<String> = batch.iter().map(|c| c.content.clone()).collect();
        let embeddings = embedder::embed_with_channel(
            &texts,
            emb_model,
            &main_repo,
            kb.embedding_channel_id.as_deref(),
        ).await?;

        // Validate embedding dimensions
        if let Some(dim) = expected_dim {
            for (i, emb) in embeddings.iter().enumerate() {
                if emb.len() != dim {
                    tracing::warn!(
                        "Embedding dim mismatch in batch {}: expected {}, got {} (chunk {})",
                        batch_done, dim, emb.len(), i
                    );
                }
            }
        }

        all_embeddings.extend(embeddings);
        batch_done += 1;
        // Embedding progress: 20% ~ 80%
        let pct = 20 + ((batch_done as f64 / total_batches as f64) * 60.0) as u8;
        emit_progress(
            app, task_id, doc_id, kb_id, filename, "embedding", pct,
            &format!("向量化 {}/{}", batch_done, total_batches),
        );
    }

    // Auto-detect and update KB embedding dimension if not set
    if expected_dim.is_none() && !all_embeddings.is_empty() {
        let detected_dim = all_embeddings[0].len() as i64;
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
                app, task_id, doc_id, kb_id, filename, "storing", pct,
                &format!("存储切片 {}/{}", i + 1, chunks_total),
            );
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
    emit_progress(app, task_id, doc_id, kb_id, filename, "finalizing", 98, "更新统计");
    repo.replace_document_chunks(kb_id, doc_id, &inserts, total_tokens)
        .await
        .map_err(|e| e.to_string())?;

    // 6. Rebuild HNSW index (best-effort, non-blocking on failure)
    emit_progress(app, task_id, doc_id, kb_id, filename, "indexing", 99, "更新向量索引");
    let pool_clone = pool.clone();
    let kb_id_clone = kb_id.to_string();
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(async {
            if let Err(e) = retriever::build_index(&pool_clone, &kb_id_clone, &app_clone).await {
                if e == retriever::INDEX_BUILD_ALREADY_RUNNING {
                    tracing::debug!(knowledge_base_id = %kb_id_clone, "index build already running");
                    return;
                }
                tracing::warn!("Failed to rebuild HNSW index for KB {} after doc: {}", kb_id_clone, e);
                let _ = app_clone.emit("kb-index-progress", serde_json::json!({
                    "kb_id": &kb_id_clone,
                    "status": "error",
                    "message": "索引构建失败"
                }));
            } else {
                let _ = app_clone.emit("kb-index-progress", serde_json::json!({
                    "kb_id": &kb_id_clone,
                    "status": "ready",
                    "message": "索引构建完成"
                }));
            }
        });
    });

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
    run_reindex_task(pool, app, doc, task.id).await
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
        if let Err(error) = run_reindex_task(&pool, &app, doc, task.id).await {
            tracing::error!(%error, "knowledge document reindex failed");
        }
    });
    Ok(task_id)
}

async fn run_reindex_task(
    pool: &SqlitePool,
    app: &AppHandle,
    doc: super::models::KbDocument,
    task_id: String,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let failure_status = if doc.chunk_count > 0 { "ready" } else { "failed" };

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
    )
    .await
}
