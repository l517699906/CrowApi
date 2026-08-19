import { invoke } from "@tauri-apps/api/core";
import type { Channel, CreateChannelInput, ReorderChannelsInput, UpdateChannelInput, TestChannelResult,
    ApiKey, CreateApiKeyInput, UpdateApiKeyInput, RequestLog, RequestSecurityFinding, LogStats,
    DashboardStats, DashboardStatsInput, Settings, GetLogsInput, ServerStatus, UsageStats,
    UsageStatsInput, KnowledgeBase, KbDocument, KbConversation, KbSource, KbIndexMeta,
    ConversationMessage, KbSearchResult, KbRagAnswer, KbTag, ServiceStatus } from "../types";

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

// 知识库 API。命令名与 Tauri knowledge_base command module 保持一一对应。
export const kbApi = {
    getAll: () => invoke<KnowledgeBase[]>("get_knowledge_bases"),
    create: (input: { name: string; description?: string; embedding_model?: string }) =>
        invoke<KnowledgeBase>("create_knowledge_base", { input }),
    update: (id: string, input: Partial<{
        name: string;
        description: string;
        embedding_model: string;
        embedding_channel_id: string;
        status: number;
        mcp_enabled: number;
        chunk_size: number;
        chunk_overlap: number;
        excluded_dirs: string;
        excluded_files: string;
        included_files: string;
    }>) => invoke<KnowledgeBase>("update_knowledge_base", { id, input }),
    delete: (id: string) => invoke<void>("delete_knowledge_base", { id }),
    getDocuments: (kbId: string) => invoke<KbDocument[]>("get_kb_documents", { kbId }),
    uploadDocument: (input: { kb_id: string; filename: string; content: string }) =>
        invoke<KbDocument>("upload_kb_document", { input }),
    deleteDocument: (docId: string, kbId: string) =>
        invoke<void>("delete_kb_document", { docId, kbId }),
    reindexDocument: (docId: string) => invoke<void>("reindex_kb_document", { docId }),
    search: (input: {
        query: string;
        kb_id?: string;
        top_k?: number;
        vector_weight?: number;
        keyword_weight?: number;
        search_mode?: string;
    }) => invoke<KbSearchResult[]>("search_knowledge_base", { input }),
    ask: (input: {
        question: string;
        kb_id?: string;
        top_k?: number;
        model?: string;
        history?: ConversationMessage[];
        deep_research?: boolean;
        max_rounds?: number;
        vector_weight?: number;
        keyword_weight?: number;
        search_mode?: string;
    }) => invoke<KbRagAnswer>("ask_knowledge_base", { input }),
    getStats: (kbId: string) => invoke<Record<string, unknown>>("get_kb_stats", { kbId }),
    getConversations: (kbId: string) => invoke<KbConversation[]>("get_kb_conversations", { kbId }),
    clearConversations: (kbId: string) => invoke<void>("clear_kb_conversations", { kbId }),
    getSources: (kbId: string) => invoke<KbSource[]>("get_kb_sources", { kbId }),
    deleteSource: (sourceId: string, kbId: string) => invoke<void>("delete_kb_source", { sourceId, kbId }),
    importSource: (kbId: string, input: {
        source_type: string;
        repo_url?: string;
        branch?: string;
        token?: string;
        url?: string;
        dir_path?: string;
        excluded_dirs?: string[];
        included_files?: string[];
        max_file_size?: number;
    }) => invoke<KbSource>("import_kb_source", { kbId, input }),
    getIndexStatus: (kbId: string) => invoke<KbIndexMeta | null>("get_kb_index_status", { kbId }),
    buildIndex: (kbId: string) => invoke<void>("build_kb_index", { kbId }),
    dropIndex: (kbId: string) => invoke<void>("drop_kb_index", { kbId }),
    getTags: (kbId: string, limit?: number) => invoke<KbTag[]>("get_kb_tags", { kbId, limit }),
};

export const serviceApi = {
    getStatuses: () => invoke<ServiceStatus[]>("get_service_statuses"),
};
