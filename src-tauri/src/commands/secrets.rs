use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use crate::db::repository::{MasterKeyRotationReport, MasterKeyStatus, Repository};
use crate::AppState;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn get_master_key_status(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<MasterKeyStatus> {
    Repository::new(state.db.pool.clone())
        .master_key_status()
        .await
        .command_error("MASTER_KEY_STATUS_FAILED", "读取主密钥状态失败", true)
}

#[tauri::command]
pub async fn rotate_master_key(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<MasterKeyRotationReport> {
    let _guard = state.maintenance_lock.lock().await;
    Repository::new(state.db.pool.clone())
        .rotate_master_key()
        .await
        .map_err(|error| {
            CommandError::reported(
                "MASTER_KEY_ROTATION_FAILED",
                "轮换主密钥失败，现有密文未被修改",
                true,
                error,
            )
        })
}
