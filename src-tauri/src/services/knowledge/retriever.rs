use super::index::HnswIndex;
use super::models::{KbIndexMeta, KbTask, SearchResult, KB_INDEX_FORMAT_VERSION};
use super::repository::KbRepository;
use crate::services::tasks::{
    emit_task_event,
    models::TASK_CANCELLED,
    repository::TaskRepository,
};
use sqlx::SqlitePool;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

pub const INDEX_BUILD_ALREADY_RUNNING: &str = "KB_INDEX_BUILD_ALREADY_RUNNING";

/// Default HNSW parameters
const DEFAULT_M: usize = 16;
const DEFAULT_EF_CONSTRUCTION: usize = 200;
const DEFAULT_EF_SEARCH: usize = 50;

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

/// Get the index file path for a KB.
pub(crate) fn index_storage_dir() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("./data"))
        .join("crowapi")
        .join("hnsw_indexes");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn index_path(kb_id: &str) -> PathBuf {
    index_storage_dir().join(format!("kb_{}.hnsw", kb_id))
}

fn index_backup_path(kb_id: &str) -> PathBuf {
    index_path(kb_id).with_extension("hnsw.bak")
}

pub(crate) fn index_artifact_paths(kb_id: &str) -> Result<Vec<PathBuf>, String> {
    super::safe_path_component(kb_id, "knowledge base ID")?;
    let path = index_path(kb_id);
    let mut paths = vec![path.clone(), index_backup_path(kb_id)];
    let Some(parent) = path.parent() else {
        return Ok(paths);
    };
    let temporary_prefix = format!("kb_{}.hnsw.tmp-", kb_id);
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&temporary_prefix))
            {
                paths.push(entry.path());
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn update_fingerprint_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn index_content_fingerprint(index: &HnswIndex, config_revision: i64) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(KB_INDEX_FORMAT_VERSION.to_le_bytes());
    hasher.update(config_revision.to_le_bytes());
    hasher.update((index.dim as u64).to_le_bytes());
    hasher.update((index.len() as u64).to_le_bytes());
    for node in &index.nodes {
        let external_id = index
            .external_id(node.id)
            .ok_or_else(|| format!("HNSW node {} has no stable chunk ID", node.id))?;
        update_fingerprint_field(&mut hasher, external_id.as_bytes());
        hasher.update((node.vector.len() as u64).to_le_bytes());
        for value in &node.vector {
            hasher.update(value.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_index_bytes(bytes: &[u8], meta: &KbIndexMeta) -> Result<HnswIndex, String> {
    if meta.format_version != KB_INDEX_FORMAT_VERSION {
        return Err(format!(
            "Unsupported HNSW index format version {}",
            meta.format_version
        ));
    }
    let expected_checksum = meta
        .index_checksum
        .as_deref()
        .ok_or_else(|| "HNSW manifest is missing the file checksum".to_string())?;
    let actual_checksum = sha256_hex(bytes);
    if actual_checksum != expected_checksum {
        return Err("HNSW index checksum mismatch".to_string());
    }
    let index = HnswIndex::from_bytes(bytes)
        .map_err(|error| format!("Failed to decode HNSW index: {}", error))?;
    if !index.initialized || index.is_empty() {
        return Err("HNSW index is empty or uninitialized".to_string());
    }
    if index.dim as i64 != meta.embedding_dim || index.len() as i64 != meta.chunk_count {
        return Err("HNSW index shape does not match its manifest".to_string());
    }
    if index.external_ids.len() != index.len() {
        return Err("HNSW index has an incomplete chunk ID mapping".to_string());
    }
    let expected_fingerprint = meta
        .content_fingerprint
        .as_deref()
        .ok_or_else(|| "HNSW manifest is missing the content fingerprint".to_string())?;
    if index_content_fingerprint(&index, meta.config_revision)? != expected_fingerprint {
        return Err("HNSW index content fingerprint mismatch".to_string());
    }
    Ok(index)
}

/// Load and validate the HNSW file against its committed database manifest.
fn load_index(kb_id: &str, meta: &KbIndexMeta) -> Result<HnswIndex, String> {
    let path = index_path(kb_id);
    let backup = index_backup_path(kb_id);
    if !path.exists() && backup.exists() {
        if let Err(error) = std::fs::rename(&backup, &path) {
            tracing::warn!(%error, %kb_id, "failed to recover HNSW index backup");
        }
    }
    let stored_path = meta
        .index_path
        .as_deref()
        .ok_or_else(|| "HNSW manifest is missing the index path".to_string())?;
    if Path::new(stored_path) != path {
        return Err("HNSW manifest points to an unexpected index path".to_string());
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("Failed to read HNSW index: {}", error))?;
    validate_index_bytes(&bytes, meta)
}

async fn current_index_manifest(
    repo: &KbRepository,
    kb_id: &str,
) -> Result<Option<KbIndexMeta>, String> {
    let kb = repo
        .get_kb(kb_id)
        .await
        .map_err(|error| format!("Failed to read knowledge base: {}", error))?;
    let Some(meta) = repo
        .get_index_meta(kb_id)
        .await
        .map_err(|error| format!("Failed to read index metadata: {}", error))?
    else {
        return Ok(None);
    };
    let chunk_count = repo
        .get_chunk_count_by_kb(kb_id)
        .await
        .map_err(|error| format!("Failed to count chunks: {}", error))?;
    Ok((kb.index_status == "ready"
        && meta.status == "ready"
        && meta.indexed_revision == kb.content_revision
        && meta.format_version == KB_INDEX_FORMAT_VERSION
        && meta.config_revision == kb.config_revision
        && meta.content_fingerprint.is_some()
        && meta.index_checksum.is_some()
        && meta.chunk_count == chunk_count)
        .then_some(meta))
}

async fn mark_superseded_index(repo: &KbRepository, kb_id: &str) {
    let fallback = if repo.get_chunk_count_by_kb(kb_id).await.unwrap_or(0) > 0 {
        "stale"
    } else {
        "none"
    };
    if let Err(error) = repo.update_kb_index_status(kb_id, fallback).await {
        tracing::warn!(%error, %kb_id, "failed to mark superseded index status");
    }
    if let Err(error) = repo.update_index_meta_status(kb_id, fallback).await {
        tracing::warn!(%error, %kb_id, "failed to mark superseded index metadata");
    }
}

/// Search knowledge base by query embedding.
/// Uses HNSW index if available, falls back to linear scan.
pub async fn search(
    pool: &SqlitePool,
    kb_id: &str,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<SearchResult>, String> {
    let repo = KbRepository::new(pool.clone());
    let current_manifest = current_index_manifest(&repo, kb_id).await?;
    if current_manifest.is_none()
        && repo
            .get_kb(kb_id)
            .await
            .map_err(|error| format!("Failed to read knowledge base: {}", error))?
            .index_status
            == "ready"
    {
        mark_superseded_index(&repo, kb_id).await;
    }

    // Only a ready index whose snapshot matches the database can be loaded.
    if let Some(manifest) = current_manifest {
        let indexed_revision = manifest.indexed_revision;
        let indexed_config_revision = manifest.config_revision;
        match load_index(kb_id, &manifest) {
            Ok(index) => {
            if index.dim == query_embedding.len() {
                tracing::debug!("Using HNSW index for KB {} ({} nodes)", kb_id, index.len());
                let hnsw_results = index.search(query_embedding, top_k);

                if !hnsw_results.is_empty() {
                    let chunks = repo
                        .get_chunks_by_kb(kb_id)
                        .await
                        .map_err(|e| format!("Failed to load chunks: {}", e))?;
                    let chunks_by_id: HashMap<
                        &str,
                        &(String, String, String, Vec<u8>, String, String),
                    > = chunks.iter().map(|chunk| (chunk.0.as_str(), chunk)).collect();
                    let mapped: Vec<SearchResult> = hnsw_results
                        .iter()
                        .filter_map(|result| {
                            let chunk_id = index.external_id(result.id)?;
                            let (id, content, metadata, _embedding, filename, doc_id) =
                                chunks_by_id.get(chunk_id).copied()?;
                            Some(SearchResult {
                                chunk_id: id.clone(),
                                doc_id: doc_id.clone(),
                                filename: filename.clone(),
                                content: content.clone(),
                                score: result.score,
                                metadata: serde_json::from_str(metadata)
                                    .unwrap_or_else(|_| serde_json::json!({})),
                            })
                        })
                        .collect();

                    let manifest_is_current = current_index_manifest(&repo, kb_id)
                        .await?
                        .is_some_and(|current| {
                            current.indexed_revision == indexed_revision
                                && current.config_revision == indexed_config_revision
                        });
                    if mapped.len() == hnsw_results.len() && manifest_is_current {
                        return Ok(mapped);
                    }

                    tracing::warn!(
                        %kb_id,
                        "HNSW snapshot changed or stable chunk mapping failed; using linear search"
                    );
                }
            } else {
                tracing::warn!(
                    "HNSW index dim ({}) != query dim ({}) for KB {}, falling back to linear scan",
                    index.dim, query_embedding.len(), kb_id
                );
            }
            }
            Err(error) => {
                tracing::warn!(%error, %kb_id, "HNSW validation failed; using linear search");
                mark_superseded_index(&repo, kb_id).await;
            }
        }
    }

    // Fallback: linear scan
    linear_search(pool, kb_id, query_embedding, top_k).await
}

/// Linear scan search (original implementation).
async fn linear_search(
    pool: &SqlitePool,
    kb_id: &str,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<SearchResult>, String> {
    let repo = KbRepository::new(pool.clone());

    let chunks = repo
        .get_chunks_by_kb(kb_id)
        .await
        .map_err(|e| format!("Failed to load chunks: {}", e))?;

    if chunks.is_empty() {
        return Ok(vec![]);
    }

    let query_dim = query_embedding.len();

    let mut scored: Vec<(f32, usize)> = Vec::with_capacity(chunks.len());
    let mut dim_mismatches = 0;

    for (i, (_, _, _, emb, _, _)) in chunks.iter().enumerate() {
        let vector = decode_embedding(emb);
        if vector.len() != query_dim {
            dim_mismatches += 1;
            continue;
        }
        let score = cosine_similarity(query_embedding, &vector);
        scored.push((score, i));
    }

    if dim_mismatches > 0 {
        tracing::warn!(
            "Skipped {} chunks with mismatched embedding dimensions (expected {}) in KB {}",
            dim_mismatches, query_dim, kb_id
        );
    }

    if scored.is_empty() {
        return Ok(vec![]);
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);

    let results = scored
        .into_iter()
        .filter_map(|(score, i)| {
            let (id, content, metadata, _emb, filename, doc_id) = &chunks[i];
            let meta: serde_json::Value = serde_json::from_str(metadata).unwrap_or(serde_json::json!({}));
            Some(SearchResult {
                chunk_id: id.clone(),
                doc_id: doc_id.clone(),
                filename: filename.clone(),
                content: content.clone(),
                score,
                metadata: meta,
            })
        })
        .collect();

    Ok(results)
}

/// Search across all knowledge bases.
/// If mcp_only is true, only search KBs with mcp_enabled = 1.
pub async fn search_all(
    pool: &SqlitePool,
    query_embedding: &[f32],
    top_k: usize,
    mcp_only: bool,
) -> Result<Vec<SearchResult>, String> {
    let repo = KbRepository::new(pool.clone());

    let kbs = repo
        .get_all_kbs()
        .await
        .map_err(|e| format!("Failed to get KBs: {}", e))?;

    let active_kbs: Vec<_> = kbs
        .iter()
        .filter(|kb| kb.status == 1 && (!mcp_only || kb.mcp_enabled == 1))
        .collect();

    if active_kbs.is_empty() {
        return Ok(vec![]);
    }

    let mut all_results = Vec::new();
    for kb in &active_kbs {
        if let Ok(results) = search(pool, &kb.id, query_embedding, top_k).await {
            all_results.extend(results);
        }
    }

    all_results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_results.truncate(top_k);

    Ok(all_results)
}

/// Get the embedding dimension for a KB by checking the first valid chunk.
pub async fn detect_embedding_dim(pool: &SqlitePool, kb_id: &str) -> Result<Option<usize>, String> {
    let repo = KbRepository::new(pool.clone());
    let chunks = repo
        .get_chunks_by_kb(kb_id)
        .await
        .map_err(|e| format!("Failed to load chunks: {}", e))?;

    for (_, _, _, emb, _, _) in &chunks {
        let vector = decode_embedding(emb);
        if !vector.is_empty() {
            return Ok(Some(vector.len()));
        }
    }

    Ok(None)
}

// ════════════════════════════════════════════════════════
// Index management
// ════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexBuildOutcome {
    Ready,
    Empty,
    Superseded,
}

fn spawn_index_task(
    pool: SqlitePool,
    kb_id: String,
    app: AppHandle,
    task_id: String,
) {
    tokio::spawn(async move {
        if let Err(error) = Box::pin(run_index_task(&pool, &kb_id, &app, task_id)).await {
            tracing::error!(%error, %kb_id, "knowledge index build failed");
        }
    });
}

/// Build HNSW index for a KB from all its chunks.
/// Emits `kb-index-progress` Tauri events with percentage.
pub async fn build_index(pool: &SqlitePool, kb_id: &str, app: &AppHandle) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let task = claim_index_task(&repo, kb_id).await?;
    run_index_task(pool, kb_id, app, task.id).await
}

pub async fn build_index_with_parent(
    pool: &SqlitePool,
    kb_id: &str,
    app: &AppHandle,
    parent_task_id: &str,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let tasks = TaskRepository::new(pool.clone());
    let task = loop {
        tasks.ensure_not_cancelled(parent_task_id).await?;
        if let Some(task) = repo
            .create_task_if_idle_with_options(
                kb_id,
                None,
                "build_index",
                1,
                Some(parent_task_id),
                false,
            )
            .await
            .map_err(|error| format!("Failed to create child index task: {}", error))?
        {
            break task;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };
    run_index_task(pool, kb_id, app, task.id).await
}

pub async fn start_index_build(
    pool: &SqlitePool,
    kb_id: &str,
    app: &AppHandle,
) -> Result<String, String> {
    let repo = KbRepository::new(pool.clone());
    let task = claim_index_task(&repo, kb_id).await?;
    let task_id = task.id.clone();
    spawn_index_task(pool.clone(), kb_id.to_string(), app.clone(), task.id);
    Ok(task_id)
}

pub async fn schedule_index_build(
    pool: &SqlitePool,
    kb_id: &str,
    app: &AppHandle,
) -> Result<Option<String>, String> {
    let repo = KbRepository::new(pool.clone());
    if repo
        .get_chunk_count_by_kb(kb_id)
        .await
        .map_err(|error| format!("Failed to count indexable chunks: {}", error))?
        == 0
    {
        drop_index(pool, kb_id).await?;
        return Ok(None);
    }
    if !repo
        .index_needs_rebuild(kb_id)
        .await
        .map_err(|error| format!("Failed to inspect index freshness: {}", error))?
    {
        return Ok(None);
    }
    let task = match claim_index_task(&repo, kb_id).await {
        Ok(task) => task,
        Err(error) if error == INDEX_BUILD_ALREADY_RUNNING => return Ok(None),
        Err(error) => return Err(error),
    };
    let task_id = task.id.clone();
    spawn_index_task(pool.clone(), kb_id.to_string(), app.clone(), task.id);
    Ok(Some(task_id))
}

pub async fn start_existing_index_task(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
) -> Result<(), String> {
    let tasks = TaskRepository::new(pool.clone());
    let task = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    if task.domain != "knowledge"
        || task.task_type != "build_index"
        || task.resource_type != "knowledge_base"
        || task.status != "pending"
    {
        return Err("后台任务不是可执行的知识库索引任务".to_string());
    }
    let payload: serde_json::Value = serde_json::from_str(&task.payload_json)
        .map_err(|error| format!("知识库索引任务参数损坏: {}", error))?;
    if payload.get("payload_version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("知识库索引任务参数版本不受支持".to_string());
    }
    if payload.get("operation").and_then(serde_json::Value::as_str) != Some("build_index")
        || payload.get("kb_id").and_then(serde_json::Value::as_str)
            != Some(task.resource_id.as_str())
        || payload.get("doc_id").is_some_and(|value| !value.is_null())
    {
        return Err("知识库索引任务参数与资源不匹配".to_string());
    }
    if !tasks.claim(task_id, "preparing").await.map_err(|error| error.to_string())? {
        return Err("知识库索引任务已经开始或结束".to_string());
    }
    let kb_id = task.resource_id;
    let task_id = task.id;
    spawn_index_task(pool.clone(), kb_id, app.clone(), task_id);
    Ok(())
}

async fn claim_index_task(repo: &KbRepository, kb_id: &str) -> Result<KbTask, String> {
    repo
        .create_task_if_idle(kb_id, None, "build_index", 1)
        .await
        .map_err(|e| format!("Failed to create index task: {}", e))?
        .ok_or_else(|| INDEX_BUILD_ALREADY_RUNNING.to_string())
}

async fn run_index_task(
    pool: &SqlitePool,
    kb_id: &str,
    app: &AppHandle,
    task_id: String,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let tasks = TaskRepository::new(pool.clone());
    ensure_task_tree_not_cancelled(&tasks, &task_id).await?;
    tasks
        .update_progress(&task_id, "preparing", 0, 0, 1)
        .await
        .map_err(|error| error.to_string())?;
    if let Ok(task) = tasks.get(&task_id).await {
        emit_task_event(app, &task, Some("准备构建知识库索引"));
    }

    if let Err(error) = repo.update_kb_index_status(kb_id, "building").await {
        let message = format!("Failed to update index status: {}", error);
        let _ = repo.complete_task(&task_id, Some(&message)).await;
        return Err(message);
    }
    if let Err(error) = repo.update_index_meta_status(kb_id, "building").await {
        tracing::warn!(%error, %kb_id, "failed to mark index metadata as building");
    }

    let result = build_index_inner(pool, kb_id, app, &task_id).await;
    match &result {
        Ok(outcome) => {
            let stage = match outcome {
                IndexBuildOutcome::Ready => "completed",
                IndexBuildOutcome::Empty => "empty",
                IndexBuildOutcome::Superseded => "superseded",
            };
            let _ = tasks.update_progress(&task_id, stage, 100, 1, 1).await;
            if let Err(error) = repo.complete_task(&task_id, None).await {
                tracing::warn!(%error, %kb_id, task_id, "failed to complete index task");
            }
        }
        Err(error) if error == TASK_CANCELLED => {
            let fallback = if repo.get_chunk_count_by_kb(kb_id).await.unwrap_or(0) > 0 {
                "stale"
            } else {
                "none"
            };
            let _ = repo.update_kb_index_status(kb_id, fallback).await;
            let _ = repo.update_index_meta_status(kb_id, fallback).await;
        }
        Err(error) => {
            if let Err(status_error) = repo.update_kb_index_status(kb_id, "error").await {
                tracing::warn!(%status_error, %kb_id, "failed to persist index error status");
            }
            if let Err(status_error) = repo.update_index_meta_status(kb_id, "error").await {
                tracing::warn!(%status_error, %kb_id, "failed to persist index metadata error status");
            }
            if let Err(status_error) = repo.complete_task(&task_id, Some(error)).await {
                tracing::warn!(%status_error, %kb_id, task_id, "failed to fail index task");
            }
        }
    }
    if let Ok(task) = tasks.get(&task_id).await {
        emit_task_event(app, &task, task.error_message.as_deref());
    }
    if result.is_ok() {
        if let Err(error) = schedule_index_build(pool, kb_id, app).await {
            tracing::warn!(%error, %kb_id, "failed to schedule a follow-up index build");
        }
    }
    result.map(|_| ())
}

async fn build_index_inner(
    pool: &SqlitePool,
    kb_id: &str,
    app: &AppHandle,
    task_id: &str,
) -> Result<IndexBuildOutcome, String> {
    let repo = KbRepository::new(pool.clone());
    let tasks = TaskRepository::new(pool.clone());
    ensure_task_tree_not_cancelled(&tasks, task_id).await?;

    let snapshot = repo
        .get_index_snapshot(kb_id)
        .await
        .map_err(|e| format!("Failed to load index snapshot: {}", e))?;
    let content_revision = snapshot.content_revision;
    let config_revision = snapshot.config_revision;
    let chunks = snapshot.chunks;

    if chunks.is_empty() {
        drop_index(pool, kb_id).await?;
        return Ok(IndexBuildOutcome::Empty);
    }

    // Build (position, vector) pairs
    let mut items: Vec<(usize, Vec<f32>)> = Vec::with_capacity(chunks.len());
    let external_ids: Vec<String> = chunks.iter().map(|chunk| chunk.0.clone()).collect();
    let mut dim = 0;

    for (i, (_, _, _, emb, _, _)) in chunks.iter().enumerate() {
        let vector = decode_embedding(emb);
        if vector.is_empty() {
            return Err(format!("Chunk {} has an invalid embedding", chunks[i].0));
        }
        if dim == 0 {
            dim = vector.len();
        }
        if vector.len() != dim {
            return Err(format!(
                "Chunk {} embedding dimension {} does not match {}",
                chunks[i].0,
                vector.len(),
                dim
            ));
        }
        items.push((i, vector));
    }

    if items.is_empty() {
        return Err("No valid embeddings found".to_string());
    }

    // Create and build the index on blocking thread pool

    // Emit initial progress with total count
    let total_items = items.len();
    tasks
        .update_progress(task_id, "building", 5, 0, total_items as i64)
        .await
        .map_err(|error| error.to_string())?;
    if let Ok(task) = tasks.get(task_id).await {
        emit_task_event(app, &task, Some("构建 HNSW 索引"));
    }
    let _ = app.emit("kb-index-progress", serde_json::json!({
        "kb_id": kb_id,
        "status": "building",
        "progress": 0,
        "current": 0,
        "total": total_items,
        "message": format!("准备构建索引：{} 个切片，维度 {}", total_items, dim)
    }));

    let app_clone = app.clone();
    let kb_id_clone = kb_id.to_string();

    // CPU-intensive build runs on blocking thread pool to avoid starving async runtime
    let index = tokio::task::spawn_blocking(move || {
        let mut index = HnswIndex::new(dim, DEFAULT_M, DEFAULT_EF_CONSTRUCTION, DEFAULT_EF_SEARCH);
        index.build_with_progress(&items, |current, total| {
            let pct = if total > 0 { current * 100 / total } else { 100 };
            let _ = app_clone.emit("kb-index-progress", serde_json::json!({
                "kb_id": &kb_id_clone,
                "status": "building",
                "progress": pct,
                "current": current,
                "total": total,
                "message": format!("构建中 {}/{} ({}%)", current, total, pct)
            }));
        });
        index.set_external_ids(external_ids)?;
        Ok::<HnswIndex, String>(index)
    })
    .await
    .map_err(|e| format!("Build task panicked: {}", e))??;
    let content_fingerprint = index_content_fingerprint(&index, config_revision)?;

    ensure_task_tree_not_cancelled(&tasks, task_id).await?;
    tasks
        .update_progress(
            task_id,
            "saving",
            95,
            total_items as i64,
            total_items as i64,
        )
        .await
        .map_err(|error| error.to_string())?;

    // Save to a temporary file and only expose the complete snapshot.
    let path = index_path(kb_id);
    let temporary_path = path.with_extension(format!("hnsw.tmp-{}", uuid::Uuid::new_v4()));
    index
        .save(&temporary_path)
        .map_err(|e| format!("Failed to save index: {}", e))?;
    let index_bytes = std::fs::read(&temporary_path)
        .map_err(|error| format!("Failed to verify saved index: {}", error))?;
    let index_checksum = sha256_hex(&index_bytes);

    let latest = match repo.get_kb(kb_id).await {
        Ok(latest) => latest,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(format!("Failed to verify index revision: {}", error));
        }
    };
    if latest.content_revision != content_revision || latest.config_revision != config_revision {
        let _ = std::fs::remove_file(&temporary_path);
        mark_superseded_index(&repo, kb_id).await;
        return Ok(IndexBuildOutcome::Superseded);
    }

    if let Err(error) = replace_index_file(kb_id, &temporary_path, &path) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }

    let path_string = path
        .to_str()
        .ok_or_else(|| "Index path is not valid UTF-8".to_string())?;
    let committed = match repo
        .commit_index_snapshot(
            kb_id,
            content_revision,
            config_revision,
            dim as i64,
            total_items as i64,
            path_string,
            KB_INDEX_FORMAT_VERSION,
            &content_fingerprint,
            &index_checksum,
        )
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            return Err(format!("Failed to commit index snapshot: {}", error));
        }
    };
    if !committed {
        let _ = std::fs::remove_file(&path);
        mark_superseded_index(&repo, kb_id).await;
        return Ok(IndexBuildOutcome::Superseded);
    }

    tracing::info!(
        "HNSW index built for KB {}: {} nodes, dim {}, saved to {:?}",
        kb_id, total_items, dim, path
    );

    Ok(IndexBuildOutcome::Ready)
}

#[cfg(not(windows))]
fn replace_index_file(_kb_id: &str, temporary: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    std::fs::rename(temporary, target)
        .map_err(|e| format!("Failed to replace index file: {}", e))
}

#[cfg(windows)]
fn replace_index_file(kb_id: &str, temporary: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    let backup = index_backup_path(kb_id);
    if backup.exists() {
        std::fs::remove_file(&backup)
            .map_err(|e| format!("Failed to remove stale index backup: {}", e))?;
    }
    if target.exists() {
        std::fs::rename(target, &backup)
            .map_err(|e| format!("Failed to stage previous index: {}", e))?;
    }
    match std::fs::rename(temporary, target) {
        Ok(()) => {
            if backup.exists() {
                let _ = std::fs::remove_file(backup);
            }
            Ok(())
        }
        Err(error) => {
            if backup.exists() {
                let _ = std::fs::rename(&backup, target);
            }
            Err(format!("Failed to replace index file: {}", error))
        }
    }
}

/// Drop the HNSW index for a KB.
pub async fn drop_index(pool: &SqlitePool, kb_id: &str) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());

    let path = index_path(kb_id);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Failed to remove index file: {}", e))?;
    }
    let backup_path = index_backup_path(kb_id);
    if backup_path.exists() {
        std::fs::remove_file(&backup_path)
            .map_err(|e| format!("Failed to remove index backup: {}", e))?;
    }

    repo.mark_index_dropped(kb_id)
        .await
        .map_err(|e| format!("Failed to update index metadata: {}", e))?;

    tracing::info!("HNSW index dropped for KB {}", kb_id);

    Ok(())
}

/// Get index metadata from DB.
pub async fn get_index_status(pool: &SqlitePool, kb_id: &str) -> Result<Option<super::models::KbIndexMeta>, String> {
    let repo = KbRepository::new(pool.clone());
    repo.get_index_meta(kb_id)
        .await
        .map_err(|e| format!("Failed to get index meta: {}", e))
}

// ════════════════════════════════════════════════════════
// FTS5 hybrid search
// ════════════════════════════════════════════════════════

/// FTS5 full-text search
async fn fts5_search(
    pool: &SqlitePool,
    kb_id: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchResult>, String> {
    let fts_query = build_fts_query(query);

    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT c.id, c.content, c.metadata, d.filename, c.doc_id \
         FROM kb_chunks_fts fts \
         JOIN kb_chunks c ON fts.chunk_id = c.id \
         JOIN kb_documents d ON c.doc_id = d.id \
         JOIN kb_knowledge_bases kb ON kb.id = c.kb_id \
         WHERE c.kb_id = ? AND d.status = 'ready' \
           AND d.processed_config_revision = kb.config_revision \
           AND kb_chunks_fts MATCH ? \
         ORDER BY rank \
         LIMIT ?"
    )
    .bind(kb_id)
    .bind(&fts_query)
    .bind(top_k as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("FTS5 search failed: {}", e))?;

    let results = rows.into_iter().enumerate().map(|(idx, (id, content, metadata, filename, doc_id))| {
        let score = 1.0 / (1.0 + idx as f32 * 0.1);
        let meta: serde_json::Value = serde_json::from_str(&metadata).unwrap_or_default();
        SearchResult {
            chunk_id: id,
            doc_id,
            filename,
            content,
            score,
            metadata: meta,
        }
    }).collect();

    Ok(results)
}

/// Build FTS5 query string from user query using improved tokenization
fn build_fts_query(query: &str) -> String {
    let tokens = tokenize_query(query);

    if tokens.is_empty() {
        return query.to_string();
    }

    // FTS5: OR-connected prefix terms for broader recall
    tokens.iter()
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Tokenize a query string for FTS5 search.
/// - English/numbers: split by whitespace and punctuation, keep tokens with 2+ chars
/// - Chinese (CJK): extract continuous CJK character runs and generate 2-grams (bigrams)
/// - Mixed: process each segment independently, then merge
fn tokenize_query(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = query.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Check if CJK character
        let is_cjk = (ch >= '\u{4e00}' && ch <= '\u{9fff}')
            || (ch >= '\u{3400}' && ch <= '\u{4dbf}')
            || (ch >= '\u{f900}' && ch <= '\u{faff}');

        if is_cjk {
            // Collect continuous CJK characters
            let mut cjk_run = Vec::new();
            while i < chars.len() {
                let c = chars[i];
                let cjk = (c >= '\u{4e00}' && c <= '\u{9fff}')
                    || (c >= '\u{3400}' && c <= '\u{4dbf}')
                    || (c >= '\u{f900}' && c <= '\u{faff}');
                if !cjk {
                    break;
                }
                cjk_run.push(c);
                i += 1;
            }

            // Generate bigrams from CJK run
            if cjk_run.len() == 1 {
                // Single CJK char: use as-is
                tokens.push(cjk_run[0].to_string());
            } else {
                for w in cjk_run.windows(2) {
                    tokens.push(format!("{}{}", w[0], w[1]));
                }
            }
        } else {
            // Collect non-CJK characters as a word
            let mut word = String::new();
            while i < chars.len() {
                let c = chars[i];
                let cjk = (c >= '\u{4e00}' && c <= '\u{9fff}')
                    || (c >= '\u{3400}' && c <= '\u{4dbf}')
                    || (c >= '\u{f900}' && c <= '\u{faff}');
                if cjk {
                    break;
                }
                // Split on whitespace and common punctuation
                if c.is_whitespace() || matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | '`' | '/' | '\\' | '|' | '<' | '>') {
                    break;
                }
                word.push(c);
                i += 1;
            }

            // Only keep tokens with 2+ characters
            if word.chars().count() >= 2 {
                tokens.push(word);
            }

            // Skip whitespace/punctuation separator
            if i < chars.len() && !chars[i].is_alphanumeric() {
                i += 1;
            }
        }
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    tokens.into_iter().filter(|t| seen.insert(t.clone())).collect()
}

/// Search result with individual score breakdowns for retrieval visualization.
#[derive(Debug, Clone)]
pub struct ScoredSearchResult {
    pub result: SearchResult,
    pub vector_score: Option<f32>,
    pub keyword_score: Option<f32>,
}

/// Hybrid search: vector + FTS5 weighted merge
/// Returns results with individual score breakdowns for visualization.
pub async fn hybrid_search(
    pool: &SqlitePool,
    kb_id: &str,
    query: &str,
    query_embedding: &[f32],
    top_k: usize,
    vector_weight: f32,
    keyword_weight: f32,
) -> Result<Vec<SearchResult>, String> {
    let scored = hybrid_search_with_details(
        pool, kb_id, query, query_embedding, top_k, vector_weight, keyword_weight,
    )
    .await?;
    Ok(scored.into_iter().map(|s| s.result).collect())
}

/// Hybrid search returning detailed score breakdowns.
pub async fn hybrid_search_with_details(
    pool: &SqlitePool,
    kb_id: &str,
    query: &str,
    query_embedding: &[f32],
    top_k: usize,
    vector_weight: f32,
    keyword_weight: f32,
) -> Result<Vec<ScoredSearchResult>, String> {
    let (vector_results, keyword_results) = tokio::join!(
        search(pool, kb_id, query_embedding, top_k * 2),
        fts5_search(pool, kb_id, query, top_k * 2),
    );

    let vector_results = vector_results.unwrap_or_default();
    let keyword_results = keyword_results.unwrap_or_default();

    // Build lookup maps: chunk_id -> (SearchResult, raw_score)
    let mut vector_map: std::collections::HashMap<String, (SearchResult, f32)> = std::collections::HashMap::new();
    for r in &vector_results {
        vector_map.insert(r.chunk_id.clone(), (r.clone(), r.score));
    }

    let mut keyword_map: std::collections::HashMap<String, (SearchResult, f32)> = std::collections::HashMap::new();
    for r in &keyword_results {
        keyword_map.insert(r.chunk_id.clone(), (r.clone(), r.score));
    }

    // Collect all unique chunk IDs
    let mut all_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    all_ids.extend(vector_map.keys().cloned());
    all_ids.extend(keyword_map.keys().cloned());

    // Compute weighted scores
    let mut scored: Vec<(String, f32, Option<f32>, Option<f32>)> = Vec::new();
    for id in &all_ids {
        let v_score = vector_map.get(id).map(|(_, s)| *s);
        let k_score = keyword_map.get(id).map(|(_, s)| *s);
        let weighted = v_score.unwrap_or(0.0) * vector_weight
            + k_score.unwrap_or(0.0) * keyword_weight;
        scored.push((id.clone(), weighted, v_score, k_score));
    }

    // Sort by weighted score descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);

    // Build final results with score breakdowns
    let mut results = Vec::with_capacity(scored.len());
    for (id, final_score, v_score, k_score) in scored {
        // Prefer vector result (has embedding metadata), fallback to keyword result
        let base = vector_map.get(&id).map(|(r, _)| r.clone())
            .or_else(|| keyword_map.get(&id).map(|(r, _)| r.clone()));
        if let Some(mut r) = base {
            r.score = final_score;
            results.push(ScoredSearchResult {
                result: r,
                vector_score: v_score,
                keyword_score: k_score,
            });
        }
    }

    Ok(results)
}

/// Keyword-only search using FTS5 (no vector search).
pub async fn keyword_only_search(
    pool: &SqlitePool,
    kb_id: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchResult>, String> {
    fts5_search(pool, kb_id, query, top_k).await
}

// ════════════════════════════════════════════════════════
// Utility functions
// ════════════════════════════════════════════════════════

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn decode_embedding(blob: &[u8]) -> Vec<f32> {
    bincode::deserialize(blob).unwrap_or_default()
}

/// Encode embedding to BLOB for storage.
pub fn encode_embedding(vec: &[f32]) -> Vec<u8> {
    bincode::serialize(vec).unwrap_or_default()
}

// ════════════════════════════════════════════════════════
// Token estimation utilities
// ════════════════════════════════════════════════════════

/// Estimate token count: ~4 chars/token for ASCII, ~2 chars/token for CJK.
pub fn estimate_tokens(text: &str) -> usize {
    let ascii_chars = text.chars().filter(|c| c.is_ascii()).count();
    let non_ascii_chars = text.chars().filter(|c| !c.is_ascii()).count();
    (ascii_chars / 4) + (non_ascii_chars / 2) + 1
}

/// Get model context window limit.
pub fn get_model_context_limit(model: &str) -> usize {
    let m = model.to_lowercase();
    if m.contains("gpt-4o") {
        128_000
    } else if m.contains("gpt-4") {
        8_192
    } else if m.contains("gpt-3.5") {
        16_385
    } else if m.contains("claude-3")
        || m.contains("claude-sonnet")
        || m.contains("claude-opus")
        || m.contains("claude-haiku")
    {
        200_000
    } else if m.contains("gemini") {
        1_000_000
    } else if m.contains("deepseek") {
        64_000
    } else if m.contains("qwen") {
        32_000
    } else if m.contains("llama") {
        8_192
    } else if m.contains("mistral") || m.contains("mixtral") {
        32_000
    } else {
        8_192
    }
}

#[cfg(test)]
mod tests {
    use super::{
        index_content_fingerprint, sha256_hex, validate_index_bytes, HnswIndex,
        KbIndexMeta, KB_INDEX_FORMAT_VERSION,
    };

    fn index_and_manifest() -> (Vec<u8>, KbIndexMeta) {
        let mut index = HnswIndex::new(2, 4, 20, 10);
        index.build(&[(0, vec![1.0, 0.0]), (1, vec![0.0, 1.0])]);
        index
            .set_external_ids(vec!["chunk-a".to_string(), "chunk-b".to_string()])
            .unwrap();
        let bytes = index.to_bytes();
        let fingerprint = index_content_fingerprint(&index, 3).unwrap();
        let checksum = sha256_hex(&bytes);
        (
            bytes,
            KbIndexMeta {
                kb_id: "kb-1".to_string(),
                index_type: "hnsw".to_string(),
                embedding_dim: 2,
                chunk_count: 2,
                index_path: Some("index.hnsw".to_string()),
                built_at: Some("2026-08-21T00:00:00Z".to_string()),
                status: "ready".to_string(),
                indexed_revision: 4,
                format_version: KB_INDEX_FORMAT_VERSION,
                config_revision: 3,
                content_fingerprint: Some(fingerprint),
                index_checksum: Some(checksum),
            },
        )
    }

    #[test]
    fn validates_complete_index_manifest() {
        let (bytes, manifest) = index_and_manifest();
        let index = validate_index_bytes(&bytes, &manifest).unwrap();
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn rejects_tampered_or_truncated_index_files() {
        let (mut bytes, manifest) = index_and_manifest();
        bytes[0] ^= 0xff;
        assert!(validate_index_bytes(&bytes, &manifest)
            .unwrap_err()
            .contains("checksum"));

        let (mut bytes, mut manifest) = index_and_manifest();
        bytes.truncate(bytes.len() / 2);
        manifest.index_checksum = Some(sha256_hex(&bytes));
        assert!(validate_index_bytes(&bytes, &manifest).is_err());
    }

    #[test]
    fn rejects_old_format_and_content_mismatch() {
        let (bytes, mut manifest) = index_and_manifest();
        manifest.format_version = 0;
        assert!(validate_index_bytes(&bytes, &manifest)
            .unwrap_err()
            .contains("format version"));

        manifest.format_version = KB_INDEX_FORMAT_VERSION;
        manifest.content_fingerprint = Some("wrong".to_string());
        assert!(validate_index_bytes(&bytes, &manifest)
            .unwrap_err()
            .contains("fingerprint"));
    }
}
