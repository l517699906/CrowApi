use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::services::wiki::{
    ingest, models::AddSourceInput, project, repository::WikiRepository,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use sha2::{Digest, Sha256};

// ── Source handlers ──

pub async fn list_sources(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_sources(&id).await {
        Ok(sources) => Json(serde_json::json!({ "data": sources })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_SOURCE_LIST_FAILED",
            "读取 Wiki 来源失败",
            error,
        ).into_response(),
    }
}

pub async fn add_source(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<AddSourceInput>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());

    let content_hash = input.content.as_ref().map(|c| {
        let mut hasher = Sha256::new();
        hasher.update(c.as_bytes());
        format!("{:x}", hasher.finalize())
    });

    let file_size = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);

    let written_path = if let Some(ref content) = input.content {
        match project::write_source_file(&id, &input.filename, content.as_bytes()).await {
            Ok(path) => Some(path),
            Err(error) => return HttpError::internal(
                "WIKI_SOURCE_FILE_WRITE_FAILED",
                "保存 Wiki 来源文件失败",
                error,
            ).into_response(),
        }
    } else {
        None
    };

    let mut persisted_input = input.clone();
    if let Some(path) = &written_path {
        persisted_input.file_path = Some(path.to_string_lossy().to_string());
    }

    match repo.add_source(&id, &persisted_input, content_hash.as_deref(), file_size).await {
        Ok(s) => (StatusCode::CREATED, Json(s)).into_response(),
        Err(error) => {
            if let Some(path) = written_path {
                if let Err(cleanup_error) = tokio::fs::remove_file(&path).await {
                    tracing::warn!(%cleanup_error, path = %path.display(), "failed to clean up Wiki source file");
                }
            }
            HttpError::internal(
                "WIKI_SOURCE_CREATE_FAILED",
                "创建 Wiki 来源失败",
                error,
            ).into_response()
        }
    }
}

pub async fn delete_source(
    State(shared): State<SharedState>,
    Path((id, sid)): Path<(String, String)>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    let source = match repo.find_source(&sid).await {
        Ok(Some(source)) if source.project_id == id => source,
        Ok(Some(_)) | Ok(None) => return HttpError::not_found(
            "WIKI_SOURCE_NOT_FOUND",
            "Wiki 来源不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "WIKI_SOURCE_READ_FAILED",
            "读取 Wiki 来源失败",
            error,
        ).into_response(),
    };
    let staged = match project::stage_source_file_removal(
        &id,
        &source.filename,
        source.file_path.as_deref(),
    ).await {
        Ok(staged) => staged,
        Err(error) => return HttpError::bad_request(
            "WIKI_SOURCE_PATH_INVALID",
            error,
        ).into_response(),
    };
    if let Err(error) = repo.delete_source(&sid).await {
        if let Some(ref removal) = staged {
            if let Err(restore_error) = project::restore_staged_removal(removal).await {
                tracing::error!(%restore_error, %sid, "failed to restore staged Wiki source file");
            }
        }
        return HttpError::internal(
            "WIKI_SOURCE_DELETE_FAILED",
            "删除 Wiki 来源失败",
            error,
        ).into_response();
    }
    if let Some(removal) = staged {
        if let Err(error) = project::finalize_staged_removal(removal).await {
            return HttpError::internal(
                "WIKI_SOURCE_FILE_DELETE_FAILED",
                "删除 Wiki 来源文件失败",
                error,
            ).into_response();
        }
    }
    (StatusCode::NO_CONTENT, "").into_response()
}

pub async fn ingest_source(
    State(shared): State<SharedState>,
    Path((id, sid)): Path<(String, String)>,
) -> Response {
    let app = shared.app.clone();
    let pool = shared.state.db.pool.clone();

    let project_id = id.clone();
    let source_id = sid.clone();

    match ingest::start_ingest_source(&app, &pool, &project_id, &source_id).await {
        Ok(task_id) => Json(serde_json::json!({
            "status": "pending",
            "task_id": task_id,
        })).into_response(),
        Err(error) if error == ingest::INGEST_ALREADY_RUNNING => HttpError::conflict(
            "WIKI_INGEST_ALREADY_RUNNING",
            "该 Wiki 来源正在摄入",
        ).into_response(),
        Err(e) => {
            // Update source status to failed
            let repo = WikiRepository::new(pool);
            if let Err(error) = repo.update_source_status(&source_id, "failed", 0, Some(&e)).await {
                tracing::warn!(%error, source_id, "failed to persist Wiki source failure status");
            }
            HttpError::internal(
                "WIKI_INGEST_FAILED",
                "Wiki 来源摄入失败",
                e,
            ).into_response()
        }
    }
}

pub async fn rescan_sources(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let app = shared.app.clone();
    let pool = shared.state.db.pool.clone();
    let repo = WikiRepository::new(pool.clone());

    // Get all pending sources and ingest them
    let sources = match repo.list_sources(&id).await {
        Ok(s) => s,
        Err(error) => return HttpError::internal(
            "WIKI_SOURCE_LIST_FAILED",
            "读取 Wiki 来源失败",
            error,
        ).into_response(),
    };

    let pending: Vec<_> = sources
        .iter()
        .filter(|source| source.status == "pending" || source.status == "failed")
        .collect();
    let mut results = Vec::new();

    for source in &pending {
        match ingest::start_ingest_source(&app, &pool, &id, &source.id).await {
            Ok(task_id) => results.push(serde_json::json!({
                "source_id": source.id,
                "filename": source.filename,
                "status": "pending",
                "task_id": task_id,
            })),
            Err(error) => {
                tracing::error!(%error, source_id = %source.id, "Wiki source rescan failed");
                if let Err(status_error) = repo
                    .update_source_status(&source.id, "failed", 0, Some("摄入失败"))
                    .await
                {
                    tracing::warn!(%status_error, source_id = %source.id, "failed to persist Wiki source failure status");
                }
                results.push(serde_json::json!({
                    "source_id": source.id,
                    "filename": source.filename,
                    "status": "failed",
                    "error": {
                        "code": "WIKI_INGEST_FAILED",
                        "message": "Wiki 来源摄入失败"
                    },
                }));
            }
        }
    }

    Json(serde_json::json!({
        "status": "pending",
        "processed": pending.len(),
        "results": results,
    })).into_response()
}

