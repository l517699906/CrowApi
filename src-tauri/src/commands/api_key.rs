use crate::db::models::{ApiKey, CreateApiKeyInput};
use crate::db::repository::Repository;
use crate::AppState;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiKeyDto {
    pub id: String,
    pub name: String,
    pub key: String,
    pub status: i64,
    pub allowed_models: Vec<String>,
    pub allowed_channels: Vec<String>,
    pub quota_limit: i64,
    pub quota_used: i64,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn mask_key(key: &str) -> String {
    if key.len() <= 15 {
        return "****".to_string();
    }
    format!("{}...{}", &key[..11], &key[key.len() - 4..])
}

fn to_dto(record: ApiKey, reveal_key: bool) -> ApiKeyDto {
    ApiKeyDto {
        id: record.id,
        name: record.name,
        key: if reveal_key { record.key } else { mask_key(&record.key) },
        status: record.status,
        allowed_models: serde_json::from_str(&record.allowed_models).unwrap_or_default(),
        allowed_channels: serde_json::from_str(&record.allowed_channels).unwrap_or_default(),
        quota_limit: record.quota_limit,
        quota_used: record.quota_used,
        expires_at: record.expires_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

#[tauri::command]
pub async fn get_api_keys(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<Vec<ApiKeyDto>, String> {
    let repo = Repository::new(state.db.pool.clone());
    repo.get_all_api_keys()
        .await
        .map_err(|error| error.to_string())
        .map(|records| records.into_iter().map(|record| to_dto(record, false)).collect())
}

#[tauri::command]
pub async fn create_api_key(
    mut input: CreateApiKeyInput,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<ApiKeyDto, String> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        return Err("密钥名称不能为空".to_string());
    }
    if input.quota_limit.unwrap_or(0) < 0 {
        return Err("Token 配额不能小于 0".to_string());
    }
    if let Some(expires_at) = input.expires_at.as_deref() {
        chrono::DateTime::parse_from_rfc3339(expires_at)
            .map_err(|_| "到期时间格式无效".to_string())?;
    }

    let repo = Repository::new(state.db.pool.clone());
    repo.create_api_key(&input)
        .await
        .map_err(|error| error.to_string())
        .map(|record| to_dto(record, true))
}

#[tauri::command]
pub async fn update_api_key(
    id: String,
    status: i64,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<(), String> {
    if !matches!(status, 0 | 1) {
        return Err("密钥状态只能是 0 或 1".to_string());
    }
    let repo = Repository::new(state.db.pool.clone());
    repo.update_api_key_status(&id, status)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_api_key(
    id: String,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<(), String> {
    let repo = Repository::new(state.db.pool.clone());
    repo.delete_api_key(&id).await.map_err(|error| error.to_string())
}
