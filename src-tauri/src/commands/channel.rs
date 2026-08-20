use crate::db::models::{Channel, CreateChannelInput, UpdateChannelInput, ChannelStats};
use crate::db::repository::Repository;
use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use crate::AppState;
use crate::adaptor::{get_adaptor, ChannelConfig};
use crate::utils::validation::normalize_http_base_url;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelDto {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: String,
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub status: i64,
    pub priority: i64,
    pub weight: i64,
    pub config: serde_json::Value,
    pub model_mapping: serde_json::Value,
    pub timeout_secs: i64,
    pub created_at: String,
    pub updated_at: String,
    pub last_test_at: Option<String>,
    pub last_test_ok: Option<i64>,
}

impl From<Channel> for ChannelDto {
    fn from(c: Channel) -> Self {
        ChannelDto {
            id: c.id,
            name: c.name,
            channel_type: c.channel_type,
            base_url: c.base_url,
            api_key: mask_key(&c.api_key),
            models: serde_json::from_str(&c.models).unwrap_or_default(),
            status: c.status,
            priority: c.priority,
            weight: c.weight,
            config: serde_json::from_str(&c.config).unwrap_or(serde_json::Value::Object(Default::default())),
            model_mapping: serde_json::from_str(&c.model_mapping).unwrap_or(serde_json::Value::Object(Default::default())),
            timeout_secs: c.timeout_secs,
            created_at: c.created_at,
            updated_at: c.updated_at,
            last_test_at: c.last_test_at,
            last_test_ok: c.last_test_ok,
        }
    }
}

fn mask_key(key: &str) -> String {
    let characters: Vec<char> = key.chars().collect();
    if characters.len() <= 8 {
        return "****".to_string();
    }
    let prefix: String = characters.iter().take(4).collect();
    let suffix: String = characters
        .iter()
        .skip(characters.len().saturating_sub(4))
        .collect();
    format!("{}...{}", prefix, suffix)
}

fn to_dto(c: Channel) -> ChannelDto {
    c.into()
}

#[tauri::command]
pub async fn get_channels(state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<Vec<ChannelDto>> {
    let repo = Repository::new(state.db.pool.clone());
    let channels = repo
        .get_all_channels()
        .await
        .command_error("CHANNEL_LIST_FAILED", "读取渠道失败", true)?;
    Ok(channels.into_iter().map(to_dto).collect())
}

#[tauri::command]
pub async fn get_channel(id: String, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<ChannelDto> {
    let repo = Repository::new(state.db.pool.clone());
    let channel = repo
        .get_channel(&id)
        .await
        .command_error("CHANNEL_READ_FAILED", "读取渠道失败", true)?;
    Ok(to_dto(channel))
}

#[tauri::command]
pub async fn create_channel(input: CreateChannelInput, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<ChannelDto> {
    let mut input = input;
    input.name = input.name.trim().to_string();
    input.base_url = normalize_http_base_url(&input.base_url)
        .map_err(CommandError::validation)?;
    input.channel_type = input.channel_type.to_ascii_lowercase();
    if input.name.is_empty() || input.base_url.is_empty() || input.models.is_empty() {
        return Err(CommandError::validation("请填写渠道名称、API 地址和至少一个模型"));
    }
    validate_timeout_secs(input.timeout_secs)?;
    let repo = Repository::new(state.db.pool.clone());
    let channel = repo
        .create_channel(&input)
        .await
        .command_error("CHANNEL_CREATE_FAILED", "创建渠道失败", false)?;
    Ok(to_dto(channel))
}

#[tauri::command]
pub async fn update_channel(input: UpdateChannelInput, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<ChannelDto> {
    let mut input = input;
    if let Some(channel_type) = input.channel_type.as_mut() {
        *channel_type = channel_type.to_ascii_lowercase();
    }
    if let Some(name) = input.name.as_mut() {
        *name = name.trim().to_string();
        if name.is_empty() {
            return Err(CommandError::validation("渠道名称不能为空"));
        }
    }
    if let Some(base_url) = input.base_url.as_mut() {
        *base_url = normalize_http_base_url(base_url)
            .map_err(CommandError::validation)?;
    }
    if input.models.as_ref().is_some_and(Vec::is_empty) {
        return Err(CommandError::validation("渠道至少需要一个模型"));
    }
    if matches!(input.api_key.as_deref(), Some("")) {
        input.api_key = None;
    }
    validate_timeout_secs(input.timeout_secs)?;
    let repo = Repository::new(state.db.pool.clone());
    let channel = repo
        .update_channel(&input)
        .await
        .command_error("CHANNEL_UPDATE_FAILED", "更新渠道失败", false)?;
    Ok(to_dto(channel))
}

fn validate_timeout_secs(timeout_secs: Option<i64>) -> CommandResult<()> {
    if timeout_secs.is_some_and(|timeout| !(1..=3_600).contains(&timeout)) {
        return Err(CommandError::validation("请求超时时间必须在 1 到 3600 秒之间"));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ReorderChannelsInput {
    pub ordered_ids: Vec<String>,
}

#[tauri::command]
pub async fn reorder_channels(
    input: ReorderChannelsInput,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<()> {
    let repo = Repository::new(state.db.pool.clone());
    let channels = repo
        .get_all_channels()
        .await
        .command_error("CHANNEL_LIST_FAILED", "读取渠道失败", true)?;
    let current_ids: HashSet<&str> = channels.iter().map(|channel| channel.id.as_str()).collect();
    let ordered_ids: HashSet<&str> = input.ordered_ids.iter().map(String::as_str).collect();

    if input.ordered_ids.len() != channels.len() || ordered_ids.len() != channels.len() || ordered_ids != current_ids {
        return Err(CommandError::conflict(
            "CHANNEL_ORDER_CONFLICT",
            "渠道列表已变化，请刷新后重试",
        )
        .with_details(serde_json::json!({
            "current_count": channels.len(),
            "received_count": input.ordered_ids.len(),
        })));
    }

    repo.reorder_channels(&input.ordered_ids)
        .await
        .command_error("CHANNEL_REORDER_FAILED", "保存渠道顺序失败", false)
}

#[tauri::command]
pub async fn toggle_channel(id: String, status: i64, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<()> {
    let repo = Repository::new(state.db.pool.clone());
    repo.update_channel_status(&id, status)
        .await
        .command_error("CHANNEL_STATUS_UPDATE_FAILED", "更新渠道状态失败", false)
}

#[tauri::command]
pub async fn delete_channel(id: String, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<()> {
    let repo = Repository::new(state.db.pool.clone());
    repo.delete_channel(&id)
        .await
        .command_error("CHANNEL_DELETE_FAILED", "删除渠道失败", false)
}

#[tauri::command]
pub async fn get_channel_stats(state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<Vec<ChannelStats>> {
    let repo = Repository::new(state.db.pool.clone());
    repo.get_channel_stats()
        .await
        .command_error("CHANNEL_STATS_FAILED", "读取渠道统计失败", true)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestChannelResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
}

#[tauri::command]
pub async fn test_channel(id: String, state: tauri::State<'_, std::sync::Arc<AppState>>) -> CommandResult<TestChannelResult> {
    let repo = Repository::new(state.db.pool.clone());
    let channel = repo
        .get_channel(&id)
        .await
        .command_error("CHANNEL_READ_FAILED", "读取渠道失败", true)?;

    let config = ChannelConfig {
        base_url: channel.base_url.clone(),
        api_key: channel.api_key.clone(),
        models: serde_json::from_str(&channel.models).unwrap_or_default(),
        model_mapping: serde_json::from_str(&channel.model_mapping).unwrap_or(serde_json::Value::Object(Default::default())),
        extra: serde_json::from_str(&channel.config).unwrap_or(serde_json::Value::Object(Default::default())),
        timeout_secs: channel.timeout_secs.max(1) as u64,
    };

    let adaptor = get_adaptor(&channel.channel_type);
    let result = adaptor
        .test(&config)
        .await
        .command_error(
            "CHANNEL_TEST_FAILED",
            "渠道测试请求失败，请检查 API 地址、密钥和网络",
            true,
        )?;

    repo.update_channel_test_result(&id, result.success)
        .await
        .command_error("CHANNEL_TEST_RESULT_SAVE_FAILED", "保存渠道测试结果失败", true)?;

    Ok(TestChannelResult {
        success: result.success,
        message: result.message,
        latency_ms: result.latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::{mask_key, to_dto, validate_timeout_secs};
    use crate::db::models::Channel;

    fn channel() -> Channel {
        Channel {
            id: "channel-1".to_string(),
            name: "Primary".to_string(),
            channel_type: "openai".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-1234567890abcdef".to_string(),
            secret_ref: None,
            api_key_last4: "cdef".to_string(),
            models: "[\"gpt-test\"]".to_string(),
            status: 1,
            priority: 10,
            weight: 1,
            config: "{}".to_string(),
            model_mapping: "{}".to_string(),
            timeout_secs: 90,
            created_at: "2026-08-20T00:00:00Z".to_string(),
            updated_at: "2026-08-20T00:00:00Z".to_string(),
            last_test_at: None,
            last_test_ok: None,
        }
    }

    #[test]
    fn channel_dto_preserves_timeout_and_masks_secret() {
        let value = serde_json::to_value(to_dto(channel())).expect("serialize channel DTO");

        assert_eq!(value["timeout_secs"], 90);
        assert_eq!(value["api_key"], "sk-1...cdef");
        assert_eq!(value["type"], "openai");
    }

    #[test]
    fn channel_masking_preserves_unicode_boundaries() {
        assert_eq!(mask_key("密钥前段1234尾部"), "密钥前段...34尾部");
    }

    #[test]
    fn channel_timeout_validation_has_stable_boundaries() {
        assert!(validate_timeout_secs(None).is_ok());
        assert!(validate_timeout_secs(Some(1)).is_ok());
        assert!(validate_timeout_secs(Some(3_600)).is_ok());
        assert!(validate_timeout_secs(Some(0)).is_err());
        assert!(validate_timeout_secs(Some(3_601)).is_err());
    }
}
