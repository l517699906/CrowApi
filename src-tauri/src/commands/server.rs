use crate::AppState;
use crate::core::error::CommandResult;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerStatus {
    pub running: bool,
    pub port: u16,
    pub url: String,
}

#[tauri::command]
pub async fn get_server_status(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>) -> CommandResult<ServerStatus> {
    let running = state.server_running.load(std::sync::atomic::Ordering::SeqCst);
    let port = *state.server_port.read().await;
    let host = crate::config::load_settings(&app).server_host;
    Ok(ServerStatus {
        running,
        port,
        url: format!("http://{}:{}", host, port),
    })
}

#[tauri::command]
pub async fn restart_server(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>) -> CommandResult<()> {
    // Stop existing server
    let mut handle_guard = state.server_handle.write().await;
    if let Some(handle) = handle_guard.take() {
        handle.abort();
        let _ = handle.await;
    }
    state.server_running.store(false, std::sync::atomic::Ordering::SeqCst);

    // Start new server
    let app_clone = app.clone();
    let state_clone = state.inner().clone();
    let new_handle = tokio::spawn(async move {
        if let Err(error) = crate::server::start_server(app_clone, state_clone).await {
            tracing::error!("CrowAPI server restart failed: {}", error);
        }
    });
    *handle_guard = Some(new_handle);

    Ok(())
}
