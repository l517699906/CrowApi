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
            .get("general.minimize_to_tray")
            .or_else(|| store.get("app.minimize_to_tray"))
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.minimize_to_tray),
        close_to_tray: store
            .get("general.close_to_tray")
            .or_else(|| store.get("app.close_to_tray"))
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.close_to_tray),
        auto_start: store
            .get("general.auto_start")
            .or_else(|| store.get("app.auto_start"))
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
