use super::{
    emit_task_event,
    models::TaskListFilter,
    repository::TaskRepository,
};
use crate::server::{error::HttpError, router::SharedState};
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};

pub async fn list_tasks(
    State(shared): State<SharedState>,
    Query(filter): Query<TaskListFilter>,
) -> Response {
    match TaskRepository::new(shared.state.db.pool.clone())
        .list(&filter)
        .await
    {
        Ok(tasks) => Json(serde_json::json!({ "data": tasks })).into_response(),
        Err(error) => HttpError::internal(
            "BACKGROUND_TASK_LIST_FAILED",
            "读取后台任务失败",
            error,
        )
        .into_response(),
    }
}

pub async fn get_task(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    match TaskRepository::new(shared.state.db.pool.clone()).get(&id).await {
        Ok(task) => Json(task).into_response(),
        Err(sqlx::Error::RowNotFound) => HttpError::not_found(
            "BACKGROUND_TASK_NOT_FOUND",
            "后台任务不存在",
        )
        .into_response(),
        Err(error) => HttpError::internal(
            "BACKGROUND_TASK_READ_FAILED",
            "读取后台任务失败",
            error,
        )
        .into_response(),
    }
}

pub async fn cancel_task(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    match TaskRepository::new(shared.state.db.pool.clone())
        .request_cancel(&id)
        .await
    {
        Ok(task) => {
            emit_task_event(&shared.app, &task, Some("已请求取消任务"));
            Json(task).into_response()
        }
        Err(sqlx::Error::RowNotFound) => HttpError::conflict(
            "BACKGROUND_TASK_NOT_CANCELLABLE",
            "任务不存在或已经结束",
        )
        .into_response(),
        Err(error) => HttpError::internal(
            "BACKGROUND_TASK_CANCEL_FAILED",
            "取消后台任务失败",
            error,
        )
        .into_response(),
    }
}

pub async fn retry_task(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    match super::dispatcher::retry_and_dispatch(
        &shared.state.db.pool,
        &shared.app,
        &id,
    )
    .await
    {
        Ok(task) => Json(task).into_response(),
        Err(error) => HttpError::conflict(
            "BACKGROUND_TASK_RETRY_FAILED",
            error,
        )
        .into_response(),
    }
}
