use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

pub const DEFAULT_KEY_QUOTA: i64 = 1_000_000;
pub const DEFAULT_TOTAL_QUOTA: i64 = 0;
pub const SUPPORTED_UI_THEMES: &[&str] = &[
    "light", "system", "dark", "mist", "ember", "graphite", "frost", "sakura", "mono",
    "ocean", "neon",
];

pub fn is_supported_ui_theme(theme: &str) -> bool {
    SUPPORTED_UI_THEMES.contains(&theme)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub server_port: u16,
    pub server_host: String,
    pub ui_theme: String,
    pub ui_language: String,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub auto_start: bool,
    pub retry_enabled: bool,
    pub retry_times: i32,
    pub default_key_quota: i64,
    pub total_quota: i64,
    pub security_enabled: bool,
    pub security_mode: String,
    pub security_scan_unicode: bool,
    pub security_scan_tools: bool,
    pub security_scan_network: bool,
    pub security_scan_response: bool,
    pub security_redact_secrets: bool,
    pub security_block_on_critical: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            server_port: 8777,
            server_host: "127.0.0.1".to_string(),
            ui_theme: "light".to_string(),
            ui_language: "zh-CN".to_string(),
            minimize_to_tray: true,
            close_to_tray: true,
            auto_start: false,
            retry_enabled: true,
            retry_times: 2,
            default_key_quota: DEFAULT_KEY_QUOTA,
            total_quota: DEFAULT_TOTAL_QUOTA,
            security_enabled: false,
            security_mode: "audit".to_string(),
            security_scan_unicode: true,
            security_scan_tools: true,
            security_scan_network: true,
            security_scan_response: false,
            security_redact_secrets: false,
            security_block_on_critical: false,
        }
    }
}

fn get_non_negative_i64(
    store: &tauri_plugin_store::Store<tauri::Wry>,
    key: &str,
    default: i64,
) -> i64 {
    store
        .get(key)
        .and_then(|value| value.as_i64())
        .filter(|value| *value >= 0)
        .unwrap_or(default)
}

pub fn load_default_key_quota(app: &AppHandle) -> i64 {
    let Ok(store) = app.store("settings.json") else {
        return DEFAULT_KEY_QUOTA;
    };
    get_non_negative_i64(&store, "quota.default_key_limit", DEFAULT_KEY_QUOTA)
}

pub fn load_total_quota(app: &AppHandle) -> i64 {
    let Ok(store) = app.store("settings.json") else {
        return DEFAULT_TOTAL_QUOTA;
    };
    get_non_negative_i64(&store, "quota.total_limit", DEFAULT_TOTAL_QUOTA)
}

pub fn load_settings(app: &AppHandle) -> AppSettings {
    let defaults = AppSettings::default();
    let Ok(store) = app.store("settings.json") else {
        return defaults;
    };

    AppSettings {
        server_port: store
            .get("server.port")
            .and_then(|value| value.as_u64())
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(defaults.server_port),
        server_host: store
            .get("server.host")
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(defaults.server_host),
        ui_theme: store
            .get("ui.theme")
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or(defaults.ui_theme),
        ui_language: store
            .get("ui.language")
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or(defaults.ui_language),
        minimize_to_tray: store
            .get("app.minimize_to_tray")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.minimize_to_tray),
        close_to_tray: store
            .get("app.close_to_tray")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.close_to_tray),
        auto_start: store
            .get("app.auto_start")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.auto_start),
        retry_enabled: store
            .get("retry.enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.retry_enabled),
        retry_times: store
            .get("retry.times")
            .and_then(|value| value.as_i64())
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(defaults.retry_times),
        default_key_quota: get_non_negative_i64(
            &store,
            "quota.default_key_limit",
            defaults.default_key_quota,
        ),
        total_quota: get_non_negative_i64(
            &store,
            "quota.total_limit",
            defaults.total_quota,
        ),
        security_enabled: store
            .get("security.enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.security_enabled),
        security_mode: store
            .get("security.mode")
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or(defaults.security_mode),
        security_scan_unicode: store
            .get("security.scan_unicode")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.security_scan_unicode),
        security_scan_tools: store
            .get("security.scan_tools")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.security_scan_tools),
        security_scan_network: store
            .get("security.scan_network")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.security_scan_network),
        security_scan_response: store
            .get("security.scan_response")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.security_scan_response),
        security_redact_secrets: store
            .get("security.redact_secrets")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.security_redact_secrets),
        security_block_on_critical: store
            .get("security.block_on_critical")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.security_block_on_critical),
    }
}

pub fn save_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let host = settings.server_host.trim();
    if host.is_empty() {
        return Err("监听地址不能为空".to_string());
    }
    if settings.server_port < 1024 {
        return Err("服务端口必须在 1024 到 65535 之间".to_string());
    }
    if !(0..=5).contains(&settings.retry_times) {
        return Err("最大重试次数必须在 0 到 5 之间".to_string());
    }
    if settings.default_key_quota < 0 || settings.total_quota < 0 {
        return Err("配额不能小于 0".to_string());
    }
    if !is_supported_ui_theme(&settings.ui_theme) {
        return Err("不支持的界面主题".to_string());
    }
    if !matches!(settings.security_mode.as_str(), "audit" | "redact" | "block") {
        return Err("安全模式必须是 audit、redact 或 block".to_string());
    }

    let store = app.store("settings.json").map_err(|error| error.to_string())?;
    store.set("server.port", serde_json::json!(settings.server_port));
    store.set("server.host", serde_json::json!(host));
    store.set("ui.theme", serde_json::json!(settings.ui_theme));
    store.set("ui.language", serde_json::json!(settings.ui_language));
    store.set("app.minimize_to_tray", serde_json::json!(settings.minimize_to_tray));
    store.set("app.close_to_tray", serde_json::json!(settings.close_to_tray));
    store.set("app.auto_start", serde_json::json!(settings.auto_start));
    store.set("retry.enabled", serde_json::json!(settings.retry_enabled));
    store.set("retry.times", serde_json::json!(settings.retry_times));
    store.set("quota.default_key_limit", serde_json::json!(settings.default_key_quota));
    store.set("quota.total_limit", serde_json::json!(settings.total_quota));
    store.set("security.enabled", serde_json::json!(settings.security_enabled));
    store.set("security.mode", serde_json::json!(settings.security_mode));
    store.set("security.scan_unicode", serde_json::json!(settings.security_scan_unicode));
    store.set("security.scan_tools", serde_json::json!(settings.security_scan_tools));
    store.set("security.scan_network", serde_json::json!(settings.security_scan_network));
    store.set("security.scan_response", serde_json::json!(settings.security_scan_response));
    store.set("security.redact_secrets", serde_json::json!(settings.security_redact_secrets));
    store.set("security.block_on_critical", serde_json::json!(settings.security_block_on_critical));
    store.save().map_err(|error| error.to_string())
}
