use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::services::wiki::repository::WikiRepository;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};

// ── Graph ──

pub async fn get_graph(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.get_graph(&id).await {
        Ok(graph) => Json(graph).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_GRAPH_READ_FAILED",
            "读取 Wiki 知识图谱失败",
            error,
        ).into_response(),
    }
}

// ── Sessions ──

pub async fn list_sessions(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_sessions(&id).await {
        Ok(sessions) => Json(serde_json::json!({ "data": sessions })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_SESSION_LIST_FAILED",
            "读取 Wiki 会话失败",
            error,
        ).into_response(),
    }
}

pub async fn clear_sessions(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    if let Err(error) = repo.clear_sessions(&id).await {
        return HttpError::internal(
            "WIKI_SESSION_CLEAR_FAILED",
            "清空 Wiki 会话失败",
            error,
        ).into_response();
    }
    (StatusCode::NO_CONTENT, "").into_response()
}

// ── Queue ──

pub async fn get_queue_status(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_tasks(&id).await {
        Ok(tasks) => Json(serde_json::json!({ "data": tasks })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_TASK_LIST_FAILED",
            "读取 Wiki 任务失败",
            error,
        ).into_response(),
    }
}

