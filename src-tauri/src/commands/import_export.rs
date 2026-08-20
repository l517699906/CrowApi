use crate::db::models::{Channel, CreateChannelInput};
use crate::db::repository::Repository;
use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use crate::utils::validation::normalize_http_base_url;
use crate::AppState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_IMPORT_BYTES: usize = 10 * 1024 * 1024;
const MAX_IMPORTED_CHANNELS: usize = 1_000;

// ─── Export types ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct CrowAPIExport {
    pub version: String,
    pub exported_at: String,
    pub r#type: String,
    #[serde(default)]
    pub secrets_included: bool,
    pub channels: Vec<ExportedChannel>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportedChannel {
    pub name: String,
    pub r#type: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_last4: Option<String>,
    pub models: Vec<String>,
    pub status: i64,
    pub priority: i64,
    pub weight: i64,
    pub config: serde_json::Value,
    pub model_mapping: serde_json::Value,
    pub timeout_secs: i64,
}

impl From<Channel> for ExportedChannel {
    fn from(c: Channel) -> Self {
        ExportedChannel {
            name: c.name,
            r#type: c.channel_type,
            base_url: c.base_url,
            api_key: None,
            api_key_last4: (!c.api_key_last4.is_empty()).then_some(c.api_key_last4),
            models: serde_json::from_str(&c.models).unwrap_or_default(),
            status: c.status,
            priority: c.priority,
            weight: c.weight,
            config: serde_json::from_str(&c.config).unwrap_or(serde_json::Value::Object(Default::default())),
            model_mapping: serde_json::from_str(&c.model_mapping).unwrap_or(serde_json::Value::Object(Default::default())),
            timeout_secs: c.timeout_secs,
        }
    }
}

// ─── Crowcode backup types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CrowcodeBackup {
    pub version: serde_json::Value,
    pub r#type: Option<String>,
    #[serde(default)]
    pub ai_settings: Option<CrowcodeAiSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CrowcodeAiSettings {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub provider_type: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub custom_models: Option<Vec<String>>,
    #[serde(default)]
    pub custom_providers: Option<Vec<CrowcodeProvider>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CrowcodeProvider {
    pub name: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub custom_models: Option<Vec<String>>,
    #[serde(default)]
    pub api_format: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

// ─── Scan result types ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ScanResult {
    pub sources: Vec<ScannedSource>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScannedSource {
    pub id: String,
    pub source: String,
    pub name: String,
    pub base_url: String,
    #[serde(default, skip_serializing)]
    pub api_key: String,
    pub key_preview: String,
    pub models: Vec<String>,
    pub api_format: String,
}

impl ScannedSource {
    fn discovered(
        source: impl Into<String>,
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        models: Vec<String>,
        api_format: impl Into<String>,
    ) -> Self {
        let source = source.into();
        let name = name.into();
        let base_url = base_url.into();
        let api_key = api_key.into();
        let api_format = api_format.into();
        let mut hasher = Sha256::new();
        hasher.update(source.as_bytes());
        hasher.update([0]);
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(base_url.as_bytes());
        hasher.update([0]);
        hasher.update(api_format.as_bytes());
        for model in &models {
            hasher.update([0]);
            hasher.update(model.as_bytes());
        }
        let id = hex::encode(&hasher.finalize()[..12]);
        let key_preview = if api_key.len() > 4 {
            format!("****{}", &api_key[api_key.len() - 4..])
        } else {
            "****".to_string()
        };

        Self {
            id,
            source,
            name,
            base_url,
            api_key,
            key_preview,
            models,
            api_format,
        }
    }
}

// ─── Import result types ────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

// ─── Commands ───────────────────────────────────────────────────────────────

/// Export all channels as a CrowAPI JSON backup
#[tauri::command]
pub async fn export_channels(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<String> {
    let repo = Repository::new(state.db.pool.clone());
    let channels = repo
        .get_all_channels()
        .await
        .command_error("CHANNEL_EXPORT_FAILED", "读取待导出渠道失败", true)?;

    let export = CrowAPIExport {
        version: "2.0".to_string(),
        exported_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        r#type: "crowapi-export".to_string(),
        secrets_included: false,
        channels: channels.into_iter().map(ExportedChannel::from).collect(),
    };

    serde_json::to_string_pretty(&export)
        .command_error("CHANNEL_EXPORT_FAILED", "生成渠道导出文件失败", false)
}

/// Import channels from a Crowcode-full-backup.json file content
#[tauri::command]
pub async fn import_crowcode_backup(
    content: String,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<ImportResult> {
    validate_import_content_size(&content)?;
    let backup: CrowcodeBackup = serde_json::from_str(&content).map_err(|error| {
        CommandError::new(
            "IMPORT_INVALID_FORMAT",
            format!("解析 Crowcode 备份文件失败: {}", error),
            false,
        )
    })?;

    let repo = Repository::new(state.db.pool.clone());
    let existing = repo
        .get_all_channels()
        .await
        .command_error("CHANNEL_LIST_FAILED", "读取现有渠道失败", true)?;
    let mut existing_names: std::collections::HashSet<String> =
        existing.iter().map(|c| c.name.clone()).collect();

    let mut imported = 0;
    let mut skipped = 0;
    let mut errors: Vec<String> = Vec::new();

    // Import main aiSettings as a channel
    if let Some(ai) = &backup.ai_settings {
        if let (Some(api_key), Some(base_url)) = (ai.api_key.as_ref(), ai.base_url.as_ref()) {
            if !api_key.is_empty() && !base_url.is_empty() {
                let name = "crowcode-default".to_string();
                if existing_names.contains(&name) {
                    skipped += 1;
                } else {
                    let models = ai.custom_models.clone().unwrap_or_default();
                    let models = if models.is_empty() {
                        ai.model.clone().into_iter().collect::<Vec<_>>()
                    } else {
                        models
                    };
                    let models = if models.is_empty() {
                        vec!["auto".to_string()]
                    } else {
                        models
                    };

                    let channel_type = guess_channel_type(
                        base_url,
                        ai.provider_type.as_deref(),
                    );

                    match normalize_http_base_url(base_url) {
                        Ok(normalized_url) => {
                            existing_names.insert(name.clone());
                            let input = CreateChannelInput {
                                name,
                                channel_type,
                                base_url: normalized_url,
                                api_key: api_key.clone(),
                                models,
                                priority: Some(0),
                                weight: Some(1),
                                config: None,
                                model_mapping: None,
                                timeout_secs: None,
                            };

                            match repo.create_channel(&input).await {
                                Ok(_) => imported += 1,
                                Err(error) => {
                                    tracing::error!(%error, "failed to import Crowcode default channel");
                                    errors.push("导入 crowcode 默认渠道失败".to_string());
                                }
                            }
                        }
                        Err(error) => {
                            skipped += 1;
                            errors.push(format!("crowcode 默认渠道地址无效: {}", error));
                        }
                    }
                }
            }
        }

        // Import custom providers
        if let Some(providers) = &ai.custom_providers {
            for p in providers {
                let name = p.name.clone();
                if existing_names.contains(&name) {
                    skipped += 1;
                    continue;
                }

                let api_key = p.api_key.clone().unwrap_or_default();
                if api_key.is_empty() && !p.base_url.contains("localhost") && !p.base_url.contains("127.0.0.1") {
                    skipped += 1;
                    continue;
                }

                let models = p.custom_models.clone().unwrap_or_default();
                let models = if models.is_empty() {
                    p.model.clone().into_iter().collect::<Vec<_>>()
                } else {
                    models
                };
                let models = if models.is_empty() {
                    vec!["auto".to_string()]
                } else {
                    models
                };

                let channel_type = guess_channel_type(
                    &p.base_url,
                    p.api_format.as_deref(),
                );

                let normalized_url = match normalize_http_base_url(&p.base_url) {
                    Ok(url) => url,
                    Err(error) => {
                        skipped += 1;
                        errors.push(format!("渠道 '{}' 地址无效: {}", p.name, error));
                        continue;
                    }
                };
                existing_names.insert(name.clone());
                let input = CreateChannelInput {
                    name,
                    channel_type,
                    base_url: normalized_url,
                    api_key,
                    models,
                    priority: Some(0),
                    weight: Some(1),
                    config: None,
                    model_mapping: None,
                    timeout_secs: None,
                };

                match repo.create_channel(&input).await {
                    Ok(_) => imported += 1,
                    Err(error) => {
                        tracing::error!(%error, channel_name = %p.name, "failed to import Crowcode channel");
                        errors.push(format!("导入渠道 '{}' 失败", p.name));
                    }
                }
            }
        }
    }

    Ok(ImportResult { imported, skipped, errors })
}

/// Import channels from a crowapi export JSON file content
#[tauri::command]
pub async fn import_crowapi_export(
    content: String,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<ImportResult> {
    validate_import_content_size(&content)?;
    let export: CrowAPIExport = serde_json::from_str(&content).map_err(|error| {
        CommandError::new(
            "IMPORT_INVALID_FORMAT",
            format!("解析 CrowAPI 导出文件失败: {}", error),
            false,
        )
    })?;
    validate_crowapi_export(&export)?;

    let repo = Repository::new(state.db.pool.clone());
    let existing = repo
        .get_all_channels()
        .await
        .command_error("CHANNEL_LIST_FAILED", "读取现有渠道失败", true)?;
    let mut existing_names: std::collections::HashSet<String> =
        existing.iter().map(|c| c.name.clone()).collect();

    let mut imported = 0;
    let mut skipped = 0;
    let mut errors: Vec<String> = Vec::new();

    for ch in export.channels {
        if existing_names.contains(&ch.name) {
            skipped += 1;
            continue;
        }

        let normalized_url = match normalize_http_base_url(&ch.base_url) {
            Ok(url) => url,
            Err(error) => {
                skipped += 1;
                errors.push(format!("渠道 '{}' 地址无效: {}", ch.name, error));
                continue;
            }
        };
        existing_names.insert(ch.name.clone());
        let input = CreateChannelInput {
            name: ch.name,
            channel_type: ch.r#type,
            base_url: normalized_url,
            api_key: ch.api_key.unwrap_or_default(),
            models: ch.models,
            priority: Some(ch.priority),
            weight: Some(ch.weight),
            config: Some(ch.config),
            model_mapping: Some(ch.model_mapping),
            timeout_secs: Some(ch.timeout_secs),
        };

        match repo.create_channel(&input).await {
            Ok(_) => imported += 1,
            Err(error) => {
                tracing::error!(%error, channel_name = %input.name, "failed to import CrowAPI channel");
                errors.push(format!("导入渠道 '{}' 失败", input.name));
            }
        }
    }

    Ok(ImportResult { imported, skipped, errors })
}

/// Scan local AI CLI tool configs (Claude Code, Codex, Cursor, etc.)
#[tauri::command]
pub async fn scan_local_ai_configs() -> CommandResult<ScanResult> {
    let home = dirs::home_dir().ok_or_else(|| CommandError::new(
        "HOME_DIRECTORY_UNAVAILABLE",
        "无法获取用户主目录",
        false,
    ))?;
    let mut sources: Vec<ScannedSource> = Vec::new();

    // 1. Claude Code: ~/.claude/settings.json
    let claude_settings = home.join(".claude").join("settings.json");
    if claude_settings.exists() {
        match std::fs::read_to_string(&claude_settings) {
            Ok(content) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(env) = json.get("env").and_then(|v| v.as_object()) {
                        let base_url = env
                            .get("ANTHROPIC_BASE_URL")
                            .and_then(|v| v.as_str())
                            .unwrap_or("https://api.anthropic.com");
                        let api_key = env
                            .get("ANTHROPIC_AUTH_TOKEN")
                            .or_else(|| env.get("ANTHROPIC_API_KEY"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let model = env
                            .get("ANTHROPIC_MODEL")
                            .and_then(|v| v.as_str())
                            .unwrap_or("claude-sonnet-4-20250514");

                        if !api_key.is_empty() {
                            sources.push(ScannedSource::discovered(
                                "claude-code",
                                "Claude Code",
                                base_url,
                                api_key,
                                vec![model.to_string()],
                                "anthropic",
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read Claude Code settings: {}", e);
            }
        }
    }

    // 2. Codex CLI: ~/.codex/config.toml or ~/.codex/config.json
    let codex_dir = home.join(".codex");
    let codex_json = codex_dir.join("config.json");
    let codex_toml = codex_dir.join("config.toml");

    if codex_json.exists() {
        if let Ok(content) = std::fs::read_to_string(&codex_json) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let base_url = json
                    .get("base_url")
                    .or_else(|| json.get("baseUrl"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://api.openai.com/v1");
                let api_key = json
                    .get("api_key")
                    .or_else(|| json.get("apiKey"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let model = json
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("gpt-4o");

                if !api_key.is_empty() {
                    sources.push(ScannedSource::discovered(
                        "codex",
                        "Codex CLI",
                        base_url,
                        api_key,
                        vec![model.to_string()],
                        "openai",
                    ));
                }
            }
        }
    } else if codex_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&codex_toml) {
            // Simple TOML parsing for known fields
            let mut base_url = String::new();
            let mut api_key = String::new();
            let mut model = String::new();

            for line in content.lines() {
                let line = line.trim();
                if let Some(val) = line.strip_prefix("base_url") {
                    base_url = val.trim_start_matches('=').trim().trim_matches('"').to_string();
                } else if let Some(val) = line.strip_prefix("api_key") {
                    api_key = val.trim_start_matches('=').trim().trim_matches('"').to_string();
                } else if let Some(val) = line.strip_prefix("model") {
                    model = val.trim_start_matches('=').trim().trim_matches('"').to_string();
                }
            }

            if !api_key.is_empty() {
                let base_url = if base_url.is_empty() {
                        "https://api.openai.com/v1".to_string()
                    } else {
                        base_url
                    };
                let models = if model.is_empty() {
                        vec!["gpt-4o".to_string()]
                    } else {
                        vec![model]
                    };

                sources.push(ScannedSource::discovered(
                    "codex",
                    "Codex CLI",
                    base_url,
                    api_key,
                    models,
                    "openai",
                ));
            }
        }
    }

    // 3. Cursor: ~/.cursor/config or ~/Library/Application Support/Cursor/User/settings.json
    let cursor_settings = home
        .join("Library")
        .join("Application Support")
        .join("Cursor")
        .join("User")
        .join("settings.json");
    if cursor_settings.exists() {
        if let Ok(content) = std::fs::read_to_string(&cursor_settings) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // Cursor may store API keys in various locations
                let base_url = json
                    .pointer("/cursorai.baseUrl")
                    .and_then(|v| v.as_str());
                let api_key = json
                    .pointer("/cursorai.apiKey")
                    .and_then(|v| v.as_str());

                if let (Some(base_url), Some(api_key)) = (base_url, api_key) {
                    if !api_key.is_empty() {
                        sources.push(ScannedSource::discovered(
                            "cursor",
                            "Cursor",
                            base_url,
                            api_key,
                            vec![],
                            "openai",
                        ));
                    }
                }
            }
        }
    }

    // 4. OpenAI CLI: ~/.openai/config.json (if exists)
    let openai_config = home.join(".openai").join("config.json");
    if openai_config.exists() {
        if let Ok(content) = std::fs::read_to_string(&openai_config) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let base_url = json
                    .get("base_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://api.openai.com/v1");
                let api_key = json
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let model = json
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("gpt-4o");

                if !api_key.is_empty() {
                    sources.push(ScannedSource::discovered(
                        "openai-cli",
                        "OpenAI CLI",
                        base_url,
                        api_key,
                        vec![model.to_string()],
                        "openai",
                    ));
                }
            }
        }
    }

    Ok(ScanResult { sources })
}

/// Import scanned sources into channels
#[tauri::command]
pub async fn import_scanned_sources(
    sources: Vec<ScannedSource>,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<ImportResult> {
    let repo = Repository::new(state.db.pool.clone());
    let selected_ids: std::collections::HashSet<String> = sources
        .into_iter()
        .map(|source| source.id)
        .filter(|id| !id.is_empty())
        .collect();
    if selected_ids.is_empty() {
        return Err(CommandError::validation("请至少选择一个要导入的配置"));
    }

    let discovered = scan_local_ai_configs().await?.sources;
    let selected: Vec<ScannedSource> = discovered
        .into_iter()
        .filter(|source| selected_ids.contains(&source.id))
        .collect();
    if selected.len() != selected_ids.len() {
        return Err(CommandError::conflict(
            "SCANNED_SOURCE_CHANGED",
            "本地配置已变化，请重新扫描后再导入",
        ));
    }

    let existing = repo
        .get_all_channels()
        .await
        .command_error("CHANNEL_LIST_FAILED", "读取现有渠道失败", true)?;
    let mut existing_names: std::collections::HashSet<String> =
        existing.iter().map(|c| c.name.clone()).collect();

    let mut imported = 0;
    let mut skipped = 0;
    let mut errors: Vec<String> = Vec::new();

    for src in selected {
        let name = src.name.clone();
        if existing_names.contains(&name) {
            skipped += 1;
            continue;
        }

        let normalized_url = match normalize_http_base_url(&src.base_url) {
            Ok(url) => url,
            Err(error) => {
                skipped += 1;
                errors.push(format!("扫描源 '{}' 地址无效: {}", src.name, error));
                continue;
            }
        };
        let channel_type = guess_channel_type(&normalized_url, Some(&src.api_format));
        existing_names.insert(name.clone());

        let input = CreateChannelInput {
            name,
            channel_type,
            base_url: normalized_url,
            api_key: src.api_key,
            models: if src.models.is_empty() {
                vec!["auto".to_string()]
            } else {
                src.models
            },
            priority: Some(0),
            weight: Some(1),
            config: None,
            model_mapping: None,
            timeout_secs: None,
        };

        match repo.create_channel(&input).await {
            Ok(_) => imported += 1,
            Err(error) => {
                tracing::error!(%error, channel_name = %input.name, "failed to import scanned channel");
                errors.push(format!("导入扫描源 '{}' 失败", input.name));
            }
        }
    }

    Ok(ImportResult { imported, skipped, errors })
}

/// Open a file dialog and return the file content (for import)
#[tauri::command]
pub async fn pick_import_file(app: tauri::AppHandle) -> CommandResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("JSON files", &["json"])
        .pick_file(move |file_path| {
            let result: Result<Option<String>, String> = match file_path {
                None => Ok(None),
                Some(file) => match file.into_path() {
                    Ok(path) => std::fs::read_to_string(&path)
                        .map(Some)
                        .map_err(|error| format!("Failed to read import file: {}", error)),
                    Err(error) => Err(format!("Invalid import file path: {}", error)),
                },
            };
            let _ = tx.send(result);
        });

    let result = rx
        .await
        .command_error("FILE_DIALOG_FAILED", "读取文件选择结果失败", false)?;
    result.map_err(|error| CommandError::reported(
        "IMPORT_FILE_READ_FAILED",
        "读取导入文件失败",
        false,
        error,
    ))
}

/// Save a file dialog and return whether save was successful (for export)
#[tauri::command]
pub async fn save_export_file(
    app: tauri::AppHandle,
    content: String,
    default_name: String,
) -> CommandResult<bool> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog()
        .file()
        .set_file_name(&default_name);
    let dialog = if default_name.to_ascii_lowercase().ends_with(".csv") {
        dialog.add_filter("CSV files", &["csv"])
    } else {
        dialog.add_filter("JSON files", &["json"])
    };
    dialog.save_file(move |file_path| {
            if let Some(path) = file_path {
                if let Some(p) = path.as_path() {
                    let result = std::fs::write(p, &content)
                        .map(|_| true)
                        .map_err(|error| format!("Failed to save export file: {}", error));
                    let _ = tx.send(result);
                    return;
                }
            }
            let _ = tx.send(Ok(false));
        });

    let result = rx
        .await
        .command_error("FILE_DIALOG_FAILED", "读取文件保存结果失败", false)?;
    result.map_err(|error| CommandError::reported(
        "EXPORT_FILE_WRITE_FAILED",
        "保存导出文件失败",
        false,
        error,
    ))
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn validate_crowapi_export(export: &CrowAPIExport) -> CommandResult<()> {
    if export.r#type != "crowapi-export" {
        return Err(CommandError::new(
            "IMPORT_TYPE_UNSUPPORTED",
            "该文件不是 CrowAPI 渠道导出文件",
            false,
        ));
    }
    if !matches!(export.version.as_str(), "1.0" | "2.0") {
        return Err(CommandError::new(
            "IMPORT_VERSION_UNSUPPORTED",
            format!("不支持的 CrowAPI 导出版本: {}", export.version),
            false,
        ));
    }
    if export.version == "2.0"
        && (export.secrets_included
            || export.channels.iter().any(|channel| channel.api_key.is_some()))
    {
        return Err(CommandError::new(
            "IMPORT_SECRET_POLICY_INVALID",
            "CrowAPI 2.0 渠道导出文件不能包含明文密钥",
            false,
        ));
    }
    if export.channels.len() > MAX_IMPORTED_CHANNELS {
        return Err(CommandError::validation(format!(
            "单次最多导入 {} 个渠道",
            MAX_IMPORTED_CHANNELS,
        )));
    }
    for channel in &export.channels {
        let name = channel.name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err(CommandError::validation("渠道名称不能为空且不能超过 128 个字符"));
        }
        normalize_http_base_url(&channel.base_url).map_err(CommandError::validation)?;
        if channel.models.is_empty() || channel.models.len() > 100 {
            return Err(CommandError::validation("每个渠道必须包含 1 到 100 个模型"));
        }
        if channel.models.iter().any(|model| model.trim().is_empty() || model.len() > 256) {
            return Err(CommandError::validation("模型名称不能为空且不能超过 256 个字符"));
        }
        if !(1..=300).contains(&channel.timeout_secs) {
            return Err(CommandError::validation("渠道超时时间必须在 1 到 300 秒之间"));
        }
    }
    Ok(())
}

fn validate_import_content_size(content: &str) -> CommandResult<()> {
    if content.len() > MAX_IMPORT_BYTES {
        return Err(CommandError::validation("导入文件不能超过 10 MB"));
    }
    Ok(())
}

fn guess_channel_type(base_url: &str, api_format: Option<&str>) -> String {
    let url = base_url.to_lowercase();

    // Check by API format first
    if let Some(fmt) = api_format {
        match fmt {
            "anthropic" => return "claude".to_string(),
            "ollama" => return "ollama".to_string(),
            _ => {}
        }
    }

    // Check by URL
    if url.contains("anthropic.com") {
        return "claude".to_string();
    }
    if url.contains("deepseek.com") {
        return "deepseek".to_string();
    }
    if url.contains("generativelanguage.googleapis.com") || url.contains("gemini") {
        return "gemini".to_string();
    }
    if url.contains("dashscope.aliyuncs.com") {
        return "qwen".to_string();
    }
    if url.contains("bigmodel.cn") {
        return "zhipu".to_string();
    }
    if url.contains("moonshot.cn") || url.contains("kimi") {
        return "moonshot".to_string();
    }
    if url.contains("volces.com") {
        return "doubao".to_string();
    }
    if url.contains("localhost:11434") || url.contains("/api/chat") {
        return "ollama".to_string();
    }

    "custom".to_string()
}

#[cfg(test)]
mod tests {
    use super::{validate_crowapi_export, CrowAPIExport, ExportedChannel, ScannedSource};
    use crate::db::models::Channel;

    fn channel_with_secret() -> Channel {
        Channel {
            id: "channel-1".to_string(),
            name: "secret channel".to_string(),
            channel_type: "openai".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-sensitive-provider-key".to_string(),
            secret_ref: Some("channel:channel-1".to_string()),
            api_key_last4: "-key".to_string(),
            models: "[\"gpt-test\"]".to_string(),
            status: 1,
            priority: 0,
            weight: 1,
            config: "{}".to_string(),
            model_mapping: "{}".to_string(),
            timeout_secs: 60,
            created_at: String::new(),
            updated_at: String::new(),
            last_test_at: None,
            last_test_ok: None,
        }
    }

    #[test]
    fn scanned_source_never_serializes_secret_material() {
        let source = ScannedSource::discovered(
            "codex",
            "Codex CLI",
            "https://api.example.com/v1",
            "sk-sensitive-secret",
            vec!["model-a".to_string()],
            "openai",
        );
        let value = serde_json::to_value(source).expect("serialize scanned source");

        assert_eq!(value["key_preview"], "****cret");
        assert!(value.get("api_key").is_none());
        assert!(!value.to_string().contains("sk-sensitive-secret"));
        assert!(value.get("raw").is_none());
    }

    #[test]
    fn channel_export_omits_provider_secret() {
        let exported = ExportedChannel::from(channel_with_secret());
        let serialized = serde_json::to_string(&exported).expect("serialize exported channel");

        assert!(exported.api_key.is_none());
        assert_eq!(exported.api_key_last4.as_deref(), Some("-key"));
        assert!(!serialized.contains("sk-sensitive-provider-key"));
        assert!(!serialized.contains("api_key\":"));
    }

    #[test]
    fn crowapi_import_rejects_unknown_type_and_version() {
        let mut export = CrowAPIExport {
            version: "1.0".to_string(),
            exported_at: "2026-08-20T00:00:00Z".to_string(),
            r#type: "not-crowapi".to_string(),
            secrets_included: true,
            channels: vec![],
        };
        assert_eq!(
            validate_crowapi_export(&export).unwrap_err().code,
            "IMPORT_TYPE_UNSUPPORTED",
        );

        export.r#type = "crowapi-export".to_string();
        export.version = "3.0".to_string();
        assert_eq!(
            validate_crowapi_export(&export).unwrap_err().code,
            "IMPORT_VERSION_UNSUPPORTED",
        );

        export.version = "2.0".to_string();
        assert_eq!(
            validate_crowapi_export(&export).unwrap_err().code,
            "IMPORT_SECRET_POLICY_INVALID",
        );
    }
}
