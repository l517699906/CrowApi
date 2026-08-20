import { invoke } from "@tauri-apps/api/core";
import type { Channel, CreateChannelInput, ReorderChannelsInput, UpdateChannelInput, TestChannelResult,
    ApiKey, CreateApiKeyInput, UpdateApiKeyInput, RequestLog, RequestSecurityFinding, LogStats,
    DashboardStats, DashboardStatsInput, Settings, GetLogsInput, ServerStatus, UsageStats,
    UsageStatsInput, KnowledgeBase, KbDocument, KbConversation, KbSource, KbIndexMeta,
    ConversationMessage, KbSearchResult, KbRagAnswer, KbTag, ServiceStatus, ChannelStats, ApiKeyStats,
    ImportResult, ScanResult, ScannedSource, BuiltinRule, CustomRule,
    CreateCustomRuleInput, UpdateBuiltinRuleInput, BackupPreview, BackupWriteResult,
    RestoreScheduleResult } from "../types";

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
    getStats: () => invoke<ChannelStats[]>("get_channel_stats"),
};

// 密钥管理 API
export const apiKeyApi = {
    getAll: () => invoke<ApiKey[]>("get_api_keys"),
    create: (input: CreateApiKeyInput) => invoke<ApiKey>("create_api_key", { input }),
    update: (input: UpdateApiKeyInput) => invoke<void>("update_api_key", { input }),
    delete: (id: string) => invoke<void>("delete_api_key", { id }),
    getStats: () => invoke<ApiKeyStats[]>("get_api_key_stats"),
};

// 日志 API（GetLogsInput 为本文件内定义的筛选参数 interface）
export const logApi = {
    getAll: (input?: GetLogsInput) => invoke<RequestLog[]>("get_logs", { input: input || {} }),
    get: (id: string) => invoke<RequestLog>("get_log", { id }),
    getSecurityFindings: (logId: string) => invoke<RequestSecurityFinding[]>("get_log_security_findings", { logId }),
    delete: (id: string) => invoke<void>("delete_log", { id }),
    getStats: (days?: number) => invoke<LogStats[]>("get_log_stats", { days }),
    deleteBefore: (beforeDate: string) => invoke<number>("delete_logs_before", { beforeDate }),
    deleteAll: () => invoke<number>("delete_all_logs"),
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

export const importExportApi = {
    exportChannels: () => invoke<string>("export_channels"),
    importCrowcodeBackup: (content: string) => invoke<ImportResult>("import_crowcode_backup", { content }),
    importCrowapiExport: (content: string) => invoke<ImportResult>("import_crowapi_export", { content }),
    scanLocalAiConfigs: () => invoke<ScanResult>("scan_local_ai_configs"),
    importScannedSources: (sources: ScannedSource[]) => (
        invoke<ImportResult>("import_scanned_sources", { sources })
    ),
    pickImportFile: () => invoke<string | null>("pick_import_file"),
};

export const backupApi = {
    create: (password: string, includeLogs: boolean) => (
        invoke<BackupWriteResult | null>("create_full_backup", { password, includeLogs })
    ),
    inspect: (password: string) => (
        invoke<BackupPreview | null>("inspect_full_backup", { password })
    ),
    scheduleRestore: (selectionId: string, password: string, keepLocalSettings: boolean) => (
        invoke<RestoreScheduleResult>("schedule_full_restore", {
            selectionId,
            password,
            keepLocalSettings,
        })
    ),
};

export const securityApi = {
    getBuiltinRules: () => invoke<BuiltinRule[]>("get_builtin_security_rules"),
    updateBuiltinRule: (id: string, input: UpdateBuiltinRuleInput) => (
        invoke<void>("update_builtin_security_rule", { id, input })
    ),
    deleteBuiltinRule: (id: string) => invoke<void>("delete_builtin_security_rule", { id }),
    resetBuiltinRules: () => invoke<BuiltinRule[]>("reset_builtin_security_rules"),
    getCustomRules: () => invoke<CustomRule[]>("get_custom_security_rules"),
    createCustomRule: (input: CreateCustomRuleInput) => (
        invoke<CustomRule>("create_custom_security_rule", { input })
    ),
    toggleCustomRule: (id: string, enabled: boolean) => (
        invoke<void>("toggle_custom_security_rule", { id, enabled })
    ),
    deleteCustomRule: (id: string) => invoke<void>("delete_custom_security_rule", { id }),
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
        embedding_batch_size: number;
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

// Wiki API：Rust 端已注册完整的项目、页面、来源与图谱命令。
export interface WikiProject {
    id: string;
    name: string;
    description: string | null;
    status: number;
    schema_text: string | null;
    wiki_dir: string;
    ingest_model: string | null;
    chat_model: string | null;
    ingest_channel_id: string | null;
    chat_channel_id: string | null;
    mcp_enabled: number;
    source_count: number;
    page_count: number;
    last_ingest_at: string | null;
    last_lint_at: string | null;
    created_at: string;
    updated_at: string;
}

export interface CreateWikiProjectInput {
    name: string;
    description?: string;
    ingest_model?: string;
    chat_model?: string;
    ingest_channel_id?: string;
    chat_channel_id?: string;
    schema_text?: string;
}

export interface UpdateWikiProjectInput {
    name?: string;
    description?: string;
    status?: number;
    schema_text?: string;
    ingest_model?: string;
    chat_model?: string;
    ingest_channel_id?: string;
    chat_channel_id?: string;
    mcp_enabled?: number;
}

export interface WikiPage {
    id: string;
    project_id: string;
    path: string;
    title: string;
    page_type: string;
    content_hash: string;
    token_count: number;
    wikilinks: string;
    frontmatter: string;
    tags: string;
    status: string;
    content?: string;
    created_at: string;
    updated_at: string;
}

export interface WikiSource {
    id: string;
    project_id: string;
    source_type: string;
    filename: string;
    file_path: string | null;
    source_url: string | null;
    content_hash: string | null;
    file_size: number;
    status: string;
    page_count: number;
    error_message: string | null;
    created_at: string;
    ingested_at: string | null;
}

export interface WikiSearchResult {
    page_id: string;
    path: string;
    title: string;
    score: number;
    snippet: string;
    page_type: string;
}

export interface WikiGraphData {
    nodes: Array<{
        id: string;
        label: string;
        path: string | null;
        node_type: string;
        link_count: number;
    }>;
    edges: Array<{
        source: string;
        target: string;
        edge_type: string;
        weight: number;
    }>;
}

export interface WikiTag {
    word: string;
    count: number;
}

export interface AddWikiSourceInput {
    source_type: string;
    filename: string;
    file_path?: string;
    source_url?: string;
    content?: string;
}

export const wikiApi = {
    getProjects: () => invoke<WikiProject[]>("get_wiki_projects"),
    createProject: (input: CreateWikiProjectInput) => invoke<WikiProject>("create_wiki_project", { input }),
    getProject: (id: string) => invoke<WikiProject>("get_wiki_project", { id }),
    updateProject: (id: string, input: UpdateWikiProjectInput) => invoke<WikiProject>("update_wiki_project", { id, input }),
    deleteProject: (id: string) => invoke<void>("delete_wiki_project", { id }),
    getPages: (projectId: string) => invoke<WikiPage[]>("get_wiki_pages", { projectId }),
    getPage: (projectId: string, path: string) => invoke<WikiPage>("get_wiki_page", { projectId, path }),
    savePage: (projectId: string, path: string, content: string) => invoke<void>("save_wiki_page", { projectId, path, content }),
    getSources: (projectId: string) => invoke<WikiSource[]>("get_wiki_sources", { projectId }),
    addSource: (projectId: string, input: AddWikiSourceInput) => invoke<WikiSource>("add_wiki_source", { projectId, input }),
    deleteSource: (sourceId: string) => invoke<void>("delete_wiki_source", { sourceId }),
    search: (projectId: string, query: string, topK?: number) => invoke<WikiSearchResult[]>("search_wiki", { projectId, query, topK }),
    getGraph: (projectId: string) => invoke<WikiGraphData>("get_wiki_graph", { projectId }),
    getStats: (projectId: string) => invoke<Record<string, unknown>>("get_wiki_stats", { projectId }),
    ingestSource: (projectId: string, sourceId: string) => invoke<{ status: string; pages_created: number; page_paths: string[] }>("ingest_wiki_source", { projectId, sourceId }),
    rescanSources: (projectId: string) => invoke<{ status: string; processed: number; results: unknown[] }>("rescan_wiki_sources", { projectId }),
    getTags: (projectId: string, limit?: number) => invoke<WikiTag[]>("get_wiki_tags", { projectId, limit }),
};

export const serviceApi = {
    getStatuses: () => invoke<ServiceStatus[]>("get_service_statuses"),
};
