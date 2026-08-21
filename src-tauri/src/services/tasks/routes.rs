use super::handlers;
use crate::server::router::SharedState;
use axum::{extract::DefaultBodyLimit, routing::get, Router};

pub fn create_router() -> Router<SharedState> {
    Router::new()
        .route("/api/tasks", get(handlers::list_tasks))
        .route("/api/tasks/{id}", get(handlers::get_task))
        .route("/api/tasks/{id}/cancel", axum::routing::post(handlers::cancel_task))
        .route("/api/tasks/{id}/retry", axum::routing::post(handlers::retry_task))
        .layer(DefaultBodyLimit::max(1024 * 1024))
}
