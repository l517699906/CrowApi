use crate::db::models::{ApiKey, CreateApiKeyInput, ApiKeyStats};
use crate::db::repository::Repository;
use crate::core::access::{normalize_access_scopes, parse_access_scopes};
use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use crate::AppState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyDto {
    pub id: String,
    pub name: String,
    pub key: String,
    pub status: i64,
    pub allowed_models: Vec<String>,
    pub allowed_channels: Vec<String>,
    pub access_scopes: Vec<String>,
    pub quota_limit: i64,
    pub quota_used: i64,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ApiKey> for ApiKeyDto {
    fn from(k: ApiKey) -> Self {
        Self::from_api_key(k, false)
    }
}

impl ApiKeyDto {
    fn from_api_key(k: ApiKey, reveal: bool) -> Self {
        let masked_key = if k.key_prefix.is_empty() {
            mask_key(&k.key)
        } else {
            format!("{}...{}", k.key_prefix, k.key_last4)
        };
        ApiKeyDto {
            id: k.id,
            name: k.name,
            key: if reveal { k.key } else { masked_key },
            status: k.status,
            allowed_models: serde_json::from_str(&k.allowed_models).unwrap_or_default(),
            allowed_channels: serde_json::from_str(&k.allowed_channels).unwrap_or_default(),
            access_scopes: parse_access_scopes(&k.access_scopes).unwrap_or_default(),
            quota_limit: k.quota_limit,
            quota_used: k.quota_used,
            expires_at: k.expires_at,
            created_at: k.created_at,
            updated_at: k.updated_at,
        }
    }
}

fn mask_key(key: &str) -> String {
    let characters: Vec<char> = key.chars().collect();
    if characters.len() <= 8 {
        return "****".to_string();
    }
    let prefix: String = characters.iter().take(12).collect();
    let suffix: String = characters
        .iter()
        .skip(characters.len().saturating_sub(4))
        .collect();
    format!("{}...{}", prefix, suffix)
}

#[tauri::command]
pub async fn get_api_keys(state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<Vec<ApiKeyDto>> {
    let repo = Repository::new(state.db.pool.clone());
    let keys = repo
        .get_all_api_keys()
        .await
        .command_error("API_KEY_LIST_FAILED", "读取访问密钥失败", true)?;
    Ok(keys.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn create_api_key(
    mut input: CreateApiKeyInput,
    app: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<ApiKeyDto> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        return Err(CommandError::validation("密钥名称不能为空"));
    }
    let quota_limit = input
        .quota_limit
        .unwrap_or_else(|| crate::config::load_default_key_quota(&app));
    if quota_limit < 0 {
        return Err(CommandError::validation("密钥配额不能小于 0"));
    }
    validate_expiration(input.expires_at.as_deref())?;
    input.access_scopes = Some(
        normalize_access_scopes(input.access_scopes.as_deref())
            .map_err(CommandError::validation)?,
    );
    input.quota_limit = Some(quota_limit);

    let repo = Repository::new(state.db.pool.clone());
    let key = repo
        .create_api_key(&input)
        .await
        .command_error("API_KEY_CREATE_FAILED", "创建访问密钥失败", false)?;
    Ok(ApiKeyDto::from_api_key(key, true))
}

#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyInput {
    pub id: String,
    pub status: Option<i64>,
    pub quota_limit: Option<i64>,
    pub expires_at: Option<String>,
    pub clear_expires_at: Option<bool>,
    pub access_scopes: Option<Vec<String>>,
}

#[tauri::command]
pub async fn update_api_key(input: UpdateApiKeyInput, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<()> {
    let repo = Repository::new(state.db.pool.clone());
    if input.status.is_none()
        && input.quota_limit.is_none()
        && input.expires_at.is_none()
        && input.clear_expires_at != Some(true)
        && input.access_scopes.is_none()
    {
        return Err(CommandError::validation("没有可更新的密钥字段"));
    }
    if let Some(status) = input.status {
        if !matches!(status, 0 | 1) {
            return Err(CommandError::validation("密钥状态只能是 0 或 1"));
        }
    }
    if input.quota_limit.is_some_and(|quota_limit| quota_limit < 0) {
        return Err(CommandError::validation("密钥配额不能小于 0"));
    }
    if input.expires_at.is_some() && input.clear_expires_at == Some(true) {
        return Err(CommandError::validation("不能同时设置和清除密钥到期时间"));
    }
    validate_expiration(input.expires_at.as_deref())?;
    let access_scopes = input
        .access_scopes
        .as_deref()
        .map(|scopes| normalize_access_scopes(Some(scopes)).map_err(CommandError::validation))
        .transpose()?;

    if let Some(status) = input.status {
        repo.update_api_key_status(&input.id, status)
            .await
            .command_error("API_KEY_UPDATE_FAILED", "更新访问密钥失败", false)?;
    }
    if let Some(quota_limit) = input.quota_limit {
        repo.update_api_key_quota(&input.id, quota_limit)
            .await
            .command_error("API_KEY_UPDATE_FAILED", "更新访问密钥失败", false)?;
    }
    if input.clear_expires_at == Some(true) || input.expires_at.is_some() {
        repo.update_api_key_expiration(&input.id, input.expires_at.as_deref())
            .await
            .command_error("API_KEY_UPDATE_FAILED", "更新访问密钥有效期失败", false)?;
    }
    if let Some(access_scopes) = access_scopes {
        repo.update_api_key_access_scopes(&input.id, &access_scopes)
            .await
            .command_error("API_KEY_UPDATE_FAILED", "更新访问密钥权限失败", false)?;
    }
    Ok(())
}

fn validate_expiration(expires_at: Option<&str>) -> CommandResult<()> {
    if let Some(expires_at) = expires_at {
        chrono::DateTime::parse_from_rfc3339(expires_at)
            .map_err(|_| CommandError::validation("密钥到期时间格式无效"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_api_key(id: String, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<()> {
    let repo = Repository::new(state.db.pool.clone());
    repo.delete_api_key(&id)
        .await
        .command_error("API_KEY_DELETE_FAILED", "删除访问密钥失败", false)
}

#[tauri::command]
pub async fn get_api_key_stats(state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<Vec<ApiKeyStats>> {
    let repo = Repository::new(state.db.pool.clone());
    repo.get_api_key_stats()
        .await
        .command_error("API_KEY_STATS_FAILED", "读取访问密钥统计失败", true)
}

#[cfg(test)]
mod tests {
    use super::{mask_key, ApiKeyDto};
    use crate::db::models::ApiKey;

    fn api_key() -> ApiKey {
        ApiKey {
            id: "key-1".to_string(),
            name: "Local client".to_string(),
            key: "sk-crowapi-1234567890abcdef".to_string(),
            key_lookup: None,
            key_hash: None,
            key_prefix: "sk-crowapi-1".to_string(),
            key_last4: "cdef".to_string(),
            status: 1,
            allowed_models: "[]".to_string(),
            allowed_channels: "[]".to_string(),
            access_scopes: "[\"gateway\"]".to_string(),
            quota_limit: 0,
            quota_used: 0,
            expires_at: None,
            created_at: "2026-08-20T00:00:00Z".to_string(),
            updated_at: "2026-08-20T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn list_dto_masks_existing_key_but_create_can_reveal_once() {
        let masked = ApiKeyDto::from(api_key());
        let revealed = ApiKeyDto::from_api_key(api_key(), true);

        assert_eq!(masked.key, "sk-crowapi-1...cdef");
        assert_eq!(revealed.key, "sk-crowapi-1234567890abcdef");
    }

    #[test]
    fn api_key_masking_preserves_unicode_boundaries() {
        assert_eq!(mask_key("访问密钥abcdefghijkl"), "访问密钥abcdefgh...ijkl");
    }
}
