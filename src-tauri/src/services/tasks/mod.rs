pub mod dispatcher;
pub mod handlers;
pub mod models;
pub mod repository;
pub mod routes;

use models::{BackgroundTask, BackgroundTaskEvent};
use tauri::{AppHandle, Emitter};

pub fn emit_task_event(app: &AppHandle, task: &BackgroundTask, detail: Option<&str>) {
    if let Err(error) = app.emit(
        "background-task-progress",
        BackgroundTaskEvent {
            task_id: &task.id,
            domain: &task.domain,
            resource_type: &task.resource_type,
            resource_id: &task.resource_id,
            subject_id: task.subject_id.as_deref(),
            parent_task_id: task.parent_task_id.as_deref(),
            status: &task.status,
            stage: &task.stage,
            progress: task.progress,
            detail,
        },
    ) {
        tracing::warn!(%error, task_id = %task.id, "failed to emit background task event");
    }
}
