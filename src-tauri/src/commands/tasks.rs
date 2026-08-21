use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use crate::services::tasks::{
    emit_task_event,
    models::{BackgroundTask, TaskListFilter},
    repository::TaskRepository,
};
use crate::AppState;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn get_background_tasks(
    state: State<'_, Arc<AppState>>,
    filter: Option<TaskListFilter>,
) -> CommandResult<Vec<BackgroundTask>> {
    TaskRepository::new(state.db.pool.clone())
        .list(&filter.unwrap_or_default())
        .await
        .command_error("BACKGROUND_TASK_LIST_FAILED", "读取后台任务失败", true)
}

#[tauri::command]
pub async fn get_background_task(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CommandResult<BackgroundTask> {
    TaskRepository::new(state.db.pool.clone())
        .get(&id)
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => {
                CommandError::new("BACKGROUND_TASK_NOT_FOUND", "后台任务不存在", false)
            }
            error => CommandError::reported(
                "BACKGROUND_TASK_READ_FAILED",
                "读取后台任务失败",
                true,
                error,
            ),
        })
}

#[tauri::command]
pub async fn cancel_background_task(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CommandResult<BackgroundTask> {
    let task = TaskRepository::new(state.db.pool.clone())
        .request_cancel(&id)
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => CommandError::new(
                "BACKGROUND_TASK_NOT_CANCELLABLE",
                "任务不存在或已经结束",
                false,
            ),
            error => CommandError::reported(
                "BACKGROUND_TASK_CANCEL_FAILED",
                "取消后台任务失败",
                true,
                error,
            ),
        })?;
    emit_task_event(&app, &task, Some("已请求取消任务"));
    Ok(task)
}

#[tauri::command]
pub async fn retry_background_task(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CommandResult<BackgroundTask> {
    crate::services::tasks::dispatcher::retry_and_dispatch(&state.db.pool, &app, &id)
        .await
        .map_err(|error| {
            CommandError::new(
                "BACKGROUND_TASK_RETRY_FAILED",
                error,
                true,
            )
        })
}
