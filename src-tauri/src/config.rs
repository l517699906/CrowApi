use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

pub const DEFAULT_KEY_QUOTA: i64 = 1_000_000;
pub const DEFAULT_TOTAL_QUOTA: i64 = 0;
pub const DEFAULT_LOG_RETENTION_DAYS: i64 = 30;
pub const DEFAULT_TASK_RETENTION_DAYS: i64 = 30;
pub const DEFAULT_ALLOWED_ORIGINS: &[&str] = &[
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://localhost:1422",
    "http://127.0.0.1:1422",
];
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
    pub allow_remote_access: bool,
    pub trusted_proxy_cidrs: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub ui_theme: String,
    pub ui_language: String,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub auto_start: bool,
    pub retry_enabled: bool,
    pub retry_times: i32,
    pub default_key_quota: i64,
    pub total_quota: i64,
    pub log_retention_days: i64,
    pub task_retention_days: i64,
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
            allow_remote_access: false,
            trusted_proxy_cidrs: Vec::new(),
            allowed_origins: default_allowed_origins(),
            ui_theme: "light".to_string(),
            ui_language: "zh-CN".to_string(),
            minimize_to_tray: true,
            close_to_tray: true,
            auto_start: false,
            retry_enabled: true,
            retry_times: 2,
            default_key_quota: DEFAULT_KEY_QUOTA,
            total_quota: DEFAULT_TOTAL_QUOTA,
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
            task_retention_days: DEFAULT_TASK_RETENTION_DAYS,
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

/// A small dependency-free CIDR representation used only for trusted proxy
/// matching. Host bits are masked on parse so equivalent networks compare
/// consistently, while family mismatches never match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpCidr {
    network: IpAddr,
    prefix_len: u8,
}

impl IpCidr {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        let (address, prefix) = value
            .split_once('/')
            .ok_or_else(|| format!("可信代理网段必须使用 CIDR 格式: {}", value))?;
        let address = IpAddr::from_str(address.trim())
            .map_err(|_| format!("可信代理网段地址无效: {}", value))?;
        let prefix_len = prefix
            .trim()
            .parse::<u8>()
            .map_err(|_| format!("可信代理网段前缀无效: {}", value))?;
        let max_prefix = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix_len > max_prefix {
            return Err(format!("可信代理网段前缀超出范围: {}", value));
        }
        let network = match address {
            IpAddr::V4(address) => {
                let bits = u32::from_be_bytes(address.octets());
                let mask = if prefix_len == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(prefix_len))
                };
                IpAddr::V4(std::net::Ipv4Addr::from((bits & mask).to_be_bytes()))
            }
            IpAddr::V6(address) => {
                let bits = u128::from_be_bytes(address.octets());
                let mask = if prefix_len == 0 {
                    0
                } else {
                    u128::MAX << (128 - u32::from(prefix_len))
                };
                IpAddr::V6(std::net::Ipv6Addr::from((bits & mask).to_be_bytes()))
            }
        };
        Ok(Self { network, prefix_len })
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let network = u32::from_be_bytes(network.octets());
                let address = u32::from_be_bytes(address.octets());
                let mask = if self.prefix_len == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(self.prefix_len))
                };
                network == (address & mask)
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let network = u128::from_be_bytes(network.octets());
                let address = u128::from_be_bytes(address.octets());
                let mask = if self.prefix_len == 0 {
                    0
                } else {
                    u128::MAX << (128 - u32::from(self.prefix_len))
                };
                network == (address & mask)
            }
            _ => false,
        }
    }
}

pub fn parse_trusted_proxy_cidrs(values: &[String]) -> Result<Vec<IpCidr>, String> {
    if values.len() > 32 {
        return Err("可信代理网段最多配置 32 条".to_string());
    }
    values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| IpCidr::parse(value))
        .collect()
}

pub fn default_allowed_origins() -> Vec<String> {
    DEFAULT_ALLOWED_ORIGINS
        .iter()
        .map(|origin| (*origin).to_string())
        .collect()
}

pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub fn server_url(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        format!("http://[{}]:{}", host, port)
    } else {
        format!("http://{}:{}", host, port)
    }
}

pub fn normalize_allowed_origins(origins: &[String]) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for origin in origins {
        let origin = origin.trim().trim_end_matches('/');
        if origin.is_empty() {
            continue;
        }
        if origin == "*" {
            return Err("跨域来源不能使用通配符".to_string());
        }
        let parsed = reqwest::Url::parse(origin)
            .map_err(|_| format!("跨域来源格式无效: {}", origin))?;
        if !matches!(parsed.scheme(), "http" | "https" | "tauri")
            || parsed.host_str().is_none()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !matches!(parsed.path(), "" | "/")
        {
            return Err(format!("跨域来源必须是 http、https 或 tauri origin: {}", origin));
        }
        let canonical = if let Some(port) = parsed.port() {
            format!("{}://{}:{}", parsed.scheme(), parsed.host_str().unwrap_or_default(), port)
        } else {
            format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or_default())
        };
        if !normalized.contains(&canonical) {
            normalized.push(canonical);
        }
    }
    Ok(normalized)
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

    let persisted_server_port = store
        .get("server.port")
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(defaults.server_port);
    // Isolated desktop checks need a per-run backend port. Keep this escape
    // hatch debug-only so a release build can never be redirected by ambient
    // environment variables.
    let server_port = if cfg!(debug_assertions) {
        std::env::var("CROWAPI_E2E_SERVER_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| *value >= 1024)
            .unwrap_or(persisted_server_port)
    } else {
        persisted_server_port
    };

    AppSettings {
        server_port,
        server_host: store
            .get("server.host")
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(defaults.server_host),
        allow_remote_access: store
            .get("server.allow_remote_access")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.allow_remote_access),
        trusted_proxy_cidrs: store
            .get("server.trusted_proxy_cidrs")
            .and_then(|value| value.as_array().cloned())
            .map(|values| {
                values
                    .into_iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .and_then(|values| parse_trusted_proxy_cidrs(&values).ok().map(|_| values))
            .unwrap_or(defaults.trusted_proxy_cidrs),
        allowed_origins: store
            .get("server.allowed_origins")
            .and_then(|value| value.as_array().cloned())
            .map(|values| {
                values
                    .into_iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .and_then(|origins| normalize_allowed_origins(&origins).ok())
            .unwrap_or(defaults.allowed_origins),
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
        log_retention_days: get_non_negative_i64(
            &store,
            "retention.log_days",
            defaults.log_retention_days,
        ),
        task_retention_days: get_non_negative_i64(
            &store,
            "retention.task_days",
            defaults.task_retention_days,
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

#[cfg(test)]
mod tests {
    use super::{
        is_loopback_host, normalize_allowed_origins, parse_trusted_proxy_cidrs, server_url, IpCidr,
    };
    use std::net::IpAddr;

    #[test]
    fn remote_binding_requires_an_explicit_non_loopback_host() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(is_loopback_host("localhost"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.8"));
    }

    #[test]
    fn cors_origins_are_exact_deduplicated_origins() {
        let origins = normalize_allowed_origins(&[
            "http://localhost:1422/".to_string(),
            "http://localhost:1422".to_string(),
            "tauri://localhost".to_string(),
        ])
        .unwrap();
        assert_eq!(origins, vec!["http://localhost:1422", "tauri://localhost"]);
        assert!(normalize_allowed_origins(&["*".to_string()]).is_err());
        assert!(normalize_allowed_origins(&["https://example.com/path".to_string()]).is_err());
    }

    #[test]
    fn server_urls_wrap_ipv6_hosts() {
        assert_eq!(server_url("127.0.0.1", 8777), "http://127.0.0.1:8777");
        assert_eq!(server_url("::1", 8777), "http://[::1]:8777");
        assert_eq!(server_url("[::1]", 8777), "http://[::1]:8777");
    }

    #[test]
    fn trusted_proxy_cidrs_match_only_their_address_family_and_range() {
        let network = IpCidr::parse("192.168.10.17/24").unwrap();
        assert!(network.contains("192.168.10.1".parse().unwrap()));
        assert!(!network.contains("192.168.11.1".parse().unwrap()));
        assert!(!network.contains("::1".parse().unwrap()));
        assert_eq!(
            network.network,
            "192.168.10.0".parse::<IpAddr>().unwrap()
        );
        assert!(IpCidr::parse("2001:db8::1/64").unwrap().contains(
            "2001:db8::abcd".parse().unwrap()
        ));
    }

    #[test]
    fn trusted_proxy_cidrs_reject_malformed_values() {
        assert!(IpCidr::parse("127.0.0.1").is_err());
        assert!(IpCidr::parse("127.0.0.1/33").is_err());
        assert!(parse_trusted_proxy_cidrs(&["not-a-network".to_string()]).is_err());
    }
}
