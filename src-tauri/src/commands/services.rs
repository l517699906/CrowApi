use tauri::State;
use std::sync::Arc;
use crate::AppState;
use crate::core::error::CommandResult;
use crate::services::{ServiceRegistry, ServiceStatus};

/// Get all service statuses (Knowledge Base, MCP, etc.)
#[tauri::command]
pub async fn get_service_statuses(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<Vec<ServiceStatus>> {
    let registry = ServiceRegistry::new();
    Ok(registry.list_status(state.inner()).await)
}
