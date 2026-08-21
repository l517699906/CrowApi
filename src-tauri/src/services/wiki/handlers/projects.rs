use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::services::wiki::models::{CreateProjectInput, UpdateProjectInput};
use crate::services::wiki::project;
use crate::services::wiki::repository::{WikiRepository, DEFAULT_SCHEMA};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};

// ── Project handlers ──

pub async fn list_projects(State(shared): State<SharedState>) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_projects().await {
        Ok(projects) => Json(serde_json::json!({ "data": projects })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PROJECT_LIST_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }
}

pub async fn create_project(
    State(shared): State<SharedState>,
    Json(input): Json<CreateProjectInput>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    let project_id = project::new_uuid();
    let schema = input.schema_text.clone().unwrap_or_else(|| {
        DEFAULT_SCHEMA.to_string()
    });

    // Create directory structure
    let dir = match project::init_project_dir(&project_id, &schema).await {
        Ok(dir) => dir,
        Err(error) => return HttpError::internal(
            "WIKI_DIRECTORY_CREATE_FAILED",
            "创建 Wiki 项目目录失败",
            error,
        ).into_response(),
    };
    let wiki_dir = dir.to_string_lossy().to_string();

    match repo.create_project(&input, &wiki_dir).await {
        Ok(p) => (StatusCode::CREATED, Json(p)).into_response(),
        Err(error) => {
            if let Err(cleanup_error) = project::remove_project_dir(&project_id).await {
                tracing::warn!(%cleanup_error, %project_id, "failed to clean up Wiki project directory");
            }
            HttpError::internal(
                "WIKI_PROJECT_CREATE_FAILED",
                "创建 Wiki 项目失败",
                error,
            ).into_response()
        }
    }
}

pub async fn get_project(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.find_project(&id).await {
        Ok(Some(project)) => Json(project).into_response(),
        Ok(None) => HttpError::not_found(
            "WIKI_PROJECT_NOT_FOUND",
            "Wiki 项目不存在",
        ).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PROJECT_READ_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }
}

pub async fn update_project(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateProjectInput>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());

    match repo.find_project(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return HttpError::not_found(
            "WIKI_PROJECT_NOT_FOUND",
            "Wiki 项目不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "WIKI_PROJECT_READ_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }

    // If schema_text changed, write to disk
    if let Some(ref schema) = input.schema_text {
        let dir = match project::project_wiki_dir(&id) {
            Ok(dir) => dir,
            Err(error) => return HttpError::bad_request(
                "WIKI_PROJECT_PATH_INVALID",
                error,
            ).into_response(),
        };
        let schema_path = dir.join("schema").join("CLAUDE.md");
        if let Err(e) = tokio::fs::write(&schema_path, schema).await {
            return HttpError::internal(
                "WIKI_SCHEMA_WRITE_FAILED",
                "保存 Wiki 结构定义失败",
                e,
            ).into_response();
        }
    }

    match repo.update_project(&id, &input).await {
        Ok(p) => Json(p).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PROJECT_UPDATE_FAILED",
            "更新 Wiki 项目失败",
            error,
        ).into_response(),
    }
}

pub async fn delete_project(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.find_project(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return HttpError::not_found(
            "WIKI_PROJECT_NOT_FOUND",
            "Wiki 项目不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "WIKI_PROJECT_READ_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }
    let staged = match project::stage_project_dir_removal(&id).await {
        Ok(staged) => staged,
        Err(error) => return HttpError::internal(
            "WIKI_DIRECTORY_STAGE_FAILED",
            "准备删除 Wiki 项目目录失败",
            error,
        ).into_response(),
    };
    if let Err(error) = repo.delete_project(&id).await {
        if let Some(ref removal) = staged {
            if let Err(restore_error) = project::restore_staged_removal(removal).await {
                tracing::error!(%restore_error, %id, "failed to restore staged Wiki project directory");
            }
        }
        return HttpError::internal(
            "WIKI_PROJECT_DELETE_FAILED",
            "删除 Wiki 项目失败",
            error,
        ).into_response();
    }
    if let Some(removal) = staged {
        if let Err(error) = project::finalize_staged_removal(removal).await {
            tracing::warn!(%error, project_id = %id, "failed to finalize Wiki project directory deletion");
        }
    }
    (StatusCode::NO_CONTENT, "").into_response()
}

pub async fn get_project_stats(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.get_stats(&id).await {
        Ok(stats) => Json(stats).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_STATS_READ_FAILED",
            "读取 Wiki 统计失败",
            error,
        ).into_response(),
    }
}

