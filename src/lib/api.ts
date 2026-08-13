import { invoke } from "@tauri-apps/api/core";
import type { Channel, CreateChannelInput, UpdateChannelInput, TestChannelResult,
    ApiKey, CreateApiKeyInput, RequestLog, LogStats, DashboardStats, Settings,
    GetLogsInput } from "../types";

// 渠道管理 API
export const channelApi = {
    getAll: () => invoke<Channel[]>("get_channels"),
    get: (id: string) => invoke<Channel>("get_channel", { id }),
    create: (input: CreateChannelInput) => invoke<Channel>("create_channel", { input }),
    update: (input: UpdateChannelInput) => invoke<Channel>("update_channel", { input }),
    toggle: (id: string, status: number) => invoke<void>("toggle_channel", { id, status }),
    delete: (id: string) => invoke<void>("delete_channel", { id }),
    test: (id: string) => invoke<TestChannelResult>("test_channel", { id }),
};

// 密钥管理 API
export const apiKeyApi = {
    getAll: () => invoke<ApiKey[]>("get_api_keys"),
    create: (input: CreateApiKeyInput) => invoke<ApiKey>("create_api_key", { input }),
    update: (id: string, status?: number) => invoke<void>("update_api_key", { input: { id, status } }),
    delete: (id: string) => invoke<void>("delete_api_key", { id }),
};

// 日志 API（GetLogsInput 为本文件内定义的筛选参数 interface）
export const logApi = {
    getAll: (input?: GetLogsInput) => invoke<RequestLog[]>("get_logs", { input: input || {} }),
    get: (id: string) => invoke<RequestLog>("get_log", { id }),
    delete: (id: string) => invoke<void>("delete_log", { id }),
    getStats: (days?: number) => invoke<LogStats[]>("get_log_stats", { days }),
};

// 仪表盘 API
export const statsApi = {
    getDashboard: () => invoke<DashboardStats>("get_dashboard_stats"),
};

// 设置 API
export const settingsApi = {
    get: () => invoke<Settings>("get_settings"),
    save: (settings: Settings) => invoke<void>("save_settings", { settings }),
};
