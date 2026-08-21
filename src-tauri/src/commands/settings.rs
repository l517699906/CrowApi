use serde::{Deserialize, Serialize};
use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_store::StoreExt;

const DEFAULT_THEME: &str = "light";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default = "default_port")]
    pub server_port: u16,
    #[serde(default = "default_host")]
    pub server_host: String,
    #[serde(default = "default_false")]
    pub allow_remote_access: bool,
    #[serde(default)]
    pub trusted_proxy_cidrs: Vec<String>,
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_theme")]
    pub ui_theme: String,
    #[serde(default = "default_language")]
    pub ui_language: String,
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default = "default_false")]
    pub auto_start: bool,
    #[serde(default = "default_retry_enabled")]
    pub retry_enabled: bool,
    #[serde(default = "default_retry_times")]
    pub retry_times: i32,
    #[serde(default = "default_key_quota")]
    pub default_key_quota: i64,
    #[serde(default = "default_total_quota")]
    pub total_quota: i64,
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: i64,
    #[serde(default = "default_task_retention_days")]
    pub task_retention_days: i64,
    #[serde(default = "default_quota_warning_threshold")]
    pub quota_warning_threshold: i64,
    #[serde(default = "default_security_enabled")]
    pub security_enabled: bool,
    #[serde(default = "default_security_mode")]
    pub security_mode: String,
    #[serde(default = "default_true")]
    pub security_scan_unicode: bool,
    #[serde(default = "default_true")]
    pub security_scan_tools: bool,
    #[serde(default = "default_true")]
    pub security_scan_network: bool,
    #[serde(default = "default_false")]
    pub security_scan_response: bool,
    #[serde(default = "default_false")]
    pub security_redact_secrets: bool,
    #[serde(default = "default_false")]
    pub security_block_on_critical: bool,
}

fn default_port() -> u16 { 8777 }
fn default_host() -> String { "127.0.0.1".to_string() }
fn default_allowed_origins() -> Vec<String> { crate::config::default_allowed_origins() }
fn default_theme() -> String { DEFAULT_THEME.to_string() }
fn default_language() -> String { "zh-CN".to_string() }
fn default_true() -> bool { true }
fn default_false() -> bool { false }
fn default_retry_enabled() -> bool { true }
fn default_retry_times() -> i32 { 2 }
fn default_key_quota() -> i64 { crate::config::DEFAULT_KEY_QUOTA }
fn default_total_quota() -> i64 { crate::config::DEFAULT_TOTAL_QUOTA }
fn default_log_retention_days() -> i64 { crate::config::DEFAULT_LOG_RETENTION_DAYS }
fn default_task_retention_days() -> i64 { crate::config::DEFAULT_TASK_RETENTION_DAYS }
fn default_quota_warning_threshold() -> i64 { 85 }
fn default_security_enabled() -> bool { true }
fn default_security_mode() -> String { "audit".to_string() }

impl Default for Settings {
    fn default() -> Self {
        Settings {
            server_port: default_port(),
            server_host: default_host(),
            allow_remote_access: default_false(),
            trusted_proxy_cidrs: Vec::new(),
            allowed_origins: default_allowed_origins(),
            ui_theme: default_theme(),
            ui_language: default_language(),
            minimize_to_tray: default_true(),
            close_to_tray: default_true(),
            auto_start: default_false(),
            retry_enabled: default_retry_enabled(),
            retry_times: default_retry_times(),
            default_key_quota: default_key_quota(),
            total_quota: default_total_quota(),
            log_retention_days: default_log_retention_days(),
            task_retention_days: default_task_retention_days(),
            quota_warning_threshold: default_quota_warning_threshold(),
            security_enabled: default_security_enabled(),
            security_mode: default_security_mode(),
            security_scan_unicode: default_true(),
            security_scan_tools: default_true(),
            security_scan_network: default_true(),
            security_scan_response: default_false(),
            security_redact_secrets: default_false(),
            security_block_on_critical: default_false(),
        }
    }
}

fn get_str(store: &tauri_plugin_store::Store<tauri::Wry>, key: &str, default: &str) -> String {
    store.get(key)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| default.to_string())
}

fn get_u64(store: &tauri_plugin_store::Store<tauri::Wry>, key: &str, default: u64) -> u64 {
    store.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
}

fn get_bool(store: &tauri_plugin_store::Store<tauri::Wry>, key: &str, default: bool) -> bool {
    store.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn get_non_negative_i64(store: &tauri_plugin_store::Store<tauri::Wry>, key: &str, default: i64) -> i64 {
    store
        .get(key)
        .and_then(|value| value.as_i64())
        .filter(|value| *value >= 0)
        .unwrap_or(default)
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> CommandResult<Settings> {
    let store = app
        .store("settings.json")
        .command_error("SETTINGS_READ_FAILED", "读取设置失败", true)?;
    let settings = Settings {
        server_port: get_u64(&store, "server.port", 8777) as u16,
        server_host: get_str(&store, "server.host", "127.0.0.1"),
        allow_remote_access: get_bool(&store, "server.allow_remote_access", false),
        trusted_proxy_cidrs: store
            .get("server.trusted_proxy_cidrs")
            .and_then(|value| value.as_array().cloned())
            .map(|values| {
                values
                    .into_iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .and_then(|values| crate::config::parse_trusted_proxy_cidrs(&values).ok().map(|_| values))
            .unwrap_or_default(),
        allowed_origins: store
            .get("server.allowed_origins")
            .and_then(|value| value.as_array().cloned())
            .map(|values| {
                values
                    .into_iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .and_then(|origins| crate::config::normalize_allowed_origins(&origins).ok())
            .unwrap_or_else(default_allowed_origins),
        ui_theme: get_str(&store, "ui.theme", DEFAULT_THEME),
        ui_language: get_str(&store, "ui.language", "zh-CN"),
        minimize_to_tray: get_bool(&store, "general.minimize_to_tray", true),
        close_to_tray: get_bool(&store, "general.close_to_tray", true),
        auto_start: get_bool(&store, "general.auto_start", false),
        retry_enabled: get_bool(&store, "retry.enabled", true),
        retry_times: get_u64(&store, "retry.times", 2) as i32,
        default_key_quota: get_non_negative_i64(
            &store,
            "quota.default_key_limit",
            crate::config::DEFAULT_KEY_QUOTA,
        ),
        total_quota: get_non_negative_i64(
            &store,
            "quota.total_limit",
            crate::config::DEFAULT_TOTAL_QUOTA,
        ),
        log_retention_days: get_non_negative_i64(
            &store,
            "retention.log_days",
            crate::config::DEFAULT_LOG_RETENTION_DAYS,
        ),
        task_retention_days: get_non_negative_i64(
            &store,
            "retention.task_days",
            crate::config::DEFAULT_TASK_RETENTION_DAYS,
        ),
        quota_warning_threshold: get_u64(&store, "quota.warning_threshold", 85).clamp(1, 100) as i64,
        security_enabled: get_bool(&store, "security.enabled", true),
        security_mode: get_str(&store, "security.mode", "audit"),
        security_scan_unicode: get_bool(&store, "security.scan_unicode", true),
        security_scan_tools: get_bool(&store, "security.scan_tools", true),
        security_scan_network: get_bool(&store, "security.scan_network", true),
        security_scan_response: get_bool(&store, "security.scan_response", false),
        security_redact_secrets: get_bool(&store, "security.redact_secrets", false),
        security_block_on_critical: get_bool(&store, "security.block_on_critical", false),
    };
    Ok(settings)
}

#[tauri::command]
pub async fn save_settings(mut settings: Settings, app: AppHandle) -> CommandResult<()> {
    let server_host = settings.server_host.trim();
    if server_host.is_empty() {
        return Err(CommandError::validation("监听地址不能为空"));
    }
    if settings.server_port < 1024 {
        return Err(CommandError::validation("服务端口必须在 1024 到 65535 之间"));
    }
    if !settings.allow_remote_access && !crate::config::is_loopback_host(server_host) {
        return Err(CommandError::validation(
            "监听非本机地址前必须显式启用远程访问",
        ));
    }
    settings.trusted_proxy_cidrs = settings
        .trusted_proxy_cidrs
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    crate::config::parse_trusted_proxy_cidrs(&settings.trusted_proxy_cidrs)
        .map_err(CommandError::validation)?;
    settings.allowed_origins = crate::config::normalize_allowed_origins(&settings.allowed_origins)
        .map_err(CommandError::validation)?;
    if !(0..=5).contains(&settings.retry_times) {
        return Err(CommandError::validation("最大重试次数必须在 0 到 5 之间"));
    }
    if settings.default_key_quota < 0 || settings.total_quota < 0 {
        return Err(CommandError::validation("配额不能小于 0"));
    }
    if !(0..=3650).contains(&settings.log_retention_days)
        || !(0..=3650).contains(&settings.task_retention_days)
    {
        return Err(CommandError::validation("数据保留天数必须在 0 到 3650 之间"));
    }
    if !(1..=100).contains(&settings.quota_warning_threshold) {
        return Err(CommandError::validation("配额告警阈值必须在 1 到 100 之间"));
    }
    if !crate::config::is_supported_ui_theme(&settings.ui_theme) {
        return Err(CommandError::validation("不支持的界面主题"));
    }
    if !matches!(settings.security_mode.as_str(), "audit" | "redact" | "block") {
        return Err(CommandError::validation("安全模式必须是 audit、redact 或 block"));
    }

    let store = app
        .store("settings.json")
        .command_error("SETTINGS_WRITE_FAILED", "保存设置失败", false)?;
    store.set("server.port", serde_json::json!(settings.server_port));
    store.set("server.host", serde_json::json!(server_host));
    store.set("server.allow_remote_access", settings.allow_remote_access);
    store.set(
        "server.trusted_proxy_cidrs",
        serde_json::json!(settings.trusted_proxy_cidrs),
    );
    store.set(
        "server.allowed_origins",
        serde_json::json!(settings.allowed_origins),
    );
    store.set("ui.theme", serde_json::json!(settings.ui_theme));
    store.set("ui.language", serde_json::json!(settings.ui_language));
    store.set("general.minimize_to_tray", settings.minimize_to_tray);
    store.set("general.close_to_tray", settings.close_to_tray);
    store.set("general.auto_start", settings.auto_start);
    store.set("retry.enabled", settings.retry_enabled);
    store.set("retry.times", settings.retry_times);
    store.set("quota.default_key_limit", serde_json::json!(settings.default_key_quota));
    store.set("quota.total_limit", serde_json::json!(settings.total_quota));
    store.set("retention.log_days", serde_json::json!(settings.log_retention_days));
    store.set("retention.task_days", serde_json::json!(settings.task_retention_days));
    store.set("quota.warning_threshold", serde_json::json!(settings.quota_warning_threshold));
    store.set("security.enabled", settings.security_enabled);
    store.set("security.mode", serde_json::json!(settings.security_mode));
    store.set("security.scan_unicode", settings.security_scan_unicode);
    store.set("security.scan_tools", settings.security_scan_tools);
    store.set("security.scan_network", settings.security_scan_network);
    store.set("security.scan_response", settings.security_scan_response);
    store.set("security.redact_secrets", settings.security_redact_secrets);
    store.set("security.block_on_critical", settings.security_block_on_critical);
    store
        .save()
        .command_error("SETTINGS_WRITE_FAILED", "保存设置失败", false)?;
    Ok(())
}

#[tauri::command]
pub async fn apply_theme(theme: String, app: AppHandle) -> CommandResult<()> {
    if let Some(window) = app.get_webview_window("main") {
        window
            .emit("theme-changed", serde_json::json!({ "theme": theme }))
            .command_error("THEME_APPLY_FAILED", "应用界面主题失败", false)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_auto_start(enabled: bool, app: AppHandle) -> CommandResult<()> {
    let autostart = app.autolaunch();
    if enabled {
        autostart
            .enable()
            .command_error("AUTOSTART_UPDATE_FAILED", "启用开机启动失败", false)?;
    } else {
        autostart
            .disable()
            .command_error("AUTOSTART_UPDATE_FAILED", "关闭开机启动失败", false)?;
    }
    Ok(())
}
