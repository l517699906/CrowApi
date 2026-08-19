import type { Settings } from "../types";

export const DEFAULT_SETTINGS: Settings = {
    server_port: 8777,
    server_host: "127.0.0.1",
    ui_theme: "light",
    ui_language: "zh-CN",
    minimize_to_tray: true,
    close_to_tray: true,
    auto_start: false,
    retry_enabled: true,
    retry_times: 2,
    default_key_quota: 1_000_000,
    total_quota: 0,
    quota_warning_threshold: 85,
    security_enabled: false,
    security_mode: "audit",
    security_scan_unicode: true,
    security_scan_tools: true,
    security_scan_network: true,
    security_scan_response: false,
    security_redact_secrets: false,
    security_block_on_critical: false,
};
