import { invoke } from "@tauri-apps/api/core";
import type { Channel, CreateChannelInput, ReorderChannelsInput, UpdateChannelInput, TestChannelResult,
    ApiKey, CreateApiKeyInput, UpdateApiKeyInput, RequestLog, RequestSecurityFinding, LogStats,
    DashboardStats, DashboardStatsInput, Settings, GetLogsInput, ServerStatus, UsageStats,
    UsageStatsInput } from "../types";

// 渠道管理 API
export const channelApi = {
    getAll: () => invoke<Channel[]>("get_channels"),
    get: (id: string) => invoke<Channel>("get_channel", { id }),
    create: (input: CreateChannelInput) => invoke<Channel>("create_channel", { input }),
    update: (input: UpdateChannelInput) => invoke<Channel>("update_channel", { input }),
    reorder: (input: ReorderChannelsInput) => invoke<void>("reorder_channels", { input }),
    toggle: (id: string, status: number) => invoke<void>("toggle_channel", { id, status }),
    delete: (id: string) => invoke<void>("delete_channel", { id }),
    test: (id: string) => invoke<TestChannelResult>("test_channel", { id }),
};

// 密钥管理 API
export const apiKeyApi = {
    getAll: () => invoke<ApiKey[]>("get_api_keys"),
    create: (input: CreateApiKeyInput) => invoke<ApiKey>("create_api_key", { input }),
    update: (input: UpdateApiKeyInput) => invoke<void>("update_api_key", { input }),
    delete: (id: string) => invoke<void>("delete_api_key", { id }),
};

// 日志 API（GetLogsInput 为本文件内定义的筛选参数 interface）
export const logApi = {
    getAll: (input?: GetLogsInput) => invoke<RequestLog[]>("get_logs", { input: input || {} }),
    get: (id: string) => invoke<RequestLog>("get_log", { id }),
    getSecurityFindings: (logId: string) => invoke<RequestSecurityFinding[]>("get_log_security_findings", { logId }),
    delete: (id: string) => invoke<void>("delete_log", { id }),
    getStats: (days?: number) => invoke<LogStats[]>("get_log_stats", { days }),
};

// 仪表盘 API
export const statsApi = {
    getDashboard: (input?: DashboardStatsInput) => invoke<DashboardStats>("get_dashboard_stats", { input: input || {} }),
    getUsage: (input?: UsageStatsInput) => (
        invoke<UsageStats>("get_usage_stats", { input: input || {} })
    ),
};

// 设置 API
export const settingsApi = {
    get: () => invoke<Settings>("get_settings"),
    save: (settings: Settings) => invoke<void>("save_settings", { settings }),
};

export const serverApi = {
    getStatus: () => invoke<ServerStatus>("get_server_status"),
    restart: () => invoke<void>("restart_server"),
};

export const fileApi = {
    saveExport: (content: string, defaultName: string) => (
        invoke<boolean>("save_export_file", { content, defaultName })
    ),
};
