export interface Channel {
    id: string;
    name: string;
    type: ChannelType;
    base_url: string;
    api_key: string;
    models: string[];
    status: number;
    priority: number;
    weight: number;
    config: Record<string, unknown>;
    model_mapping: Record<string, string | string[]>;
    timeout_secs: number;
    created_at: string;
    updated_at: string;
    last_test_at: string | null;
    last_test_ok: number | null;
}

export type ChannelType = "openai" | "deepseek" | "claude" | "gemini" | "custom";

export interface CreateChannelInput {
    name: string;
    type: ChannelType;
    base_url: string;
    api_key: string;
    models: string[];
    priority: number;
    weight: number;
    config?: Record<string, unknown>;
    model_mapping?: Record<string, string | string[]>;
    timeout_secs?: number;
}

export interface UpdateChannelInput extends Partial<CreateChannelInput> {
    id: string;
    status?: number;
    name?: string;
    base_url?: string;
    api_key?: string;
    models?: string[];
    priority?: number;
    weight?: number;
    config?: Record<string, unknown>;
    model_mapping?: Record<string, string | string[]>;
    timeout_secs?: number;
}

export interface ReorderChannelsInput {
    ordered_ids: string[];
}

export interface TestChannelResult {
    success: boolean;
    latency_ms: number;
    message: string;
    models?: string[];
}

export interface ChannelStats {
    channel_id: string;
    total_calls: number;
    success_calls: number;
    failed_calls: number;
    total_tokens: number;
    prompt_tokens: number;
    completion_tokens: number;
    avg_latency_ms: number;
    last_call_at: string | null;
}

export interface ApiKeyStats {
    api_key_id: string;
    total_calls: number;
    success_calls: number;
    failed_calls: number;
    total_tokens: number;
    prompt_tokens: number;
    completion_tokens: number;
    avg_latency_ms: number;
    last_call_at: string | null;
}

export interface ImportResult {
    imported: number;
    skipped: number;
    errors: string[];
}

export interface ScannedSource {
    id: string;
    source: string;
    name: string;
    base_url: string;
    key_preview: string;
    models: string[];
    api_format: string;
}

export interface ScanResult {
    sources: ScannedSource[];
}

export interface BackupSummary {
    createdAt: string;
    appVersion: string;
    schemaVersion: number;
    databaseBytes: number;
    fileCount: number;
    fileBytes: number;
    channelCount: number;
    apiKeyCount: number;
    knowledgeBaseCount: number;
    wikiProjectCount: number;
    includesLogs: boolean;
}

export interface BackupPreview {
    selectionId: string;
    summary: BackupSummary;
    warnings: string[];
}

export interface BackupWriteResult {
    path: string;
    summary: BackupSummary;
}

export interface RestoreScheduleResult {
    restartRequired: boolean;
    rollbackDirectory: string;
}

export type SecurityRuleSeverity = "info" | "low" | "medium" | "high" | "critical";

export interface BuiltinRule {
    id: string;
    rule_id: string;
    category: string;
    severity: SecurityRuleSeverity;
    title: string;
    description: string | null;
    toggle_key: string | null;
    enabled: number;
    created_at: string;
}

export interface CustomRule {
    id: string;
    rule_type: string;
    category: string;
    pattern: string;
    severity: SecurityRuleSeverity;
    action: string;
    enabled: number;
    description: string | null;
    created_at: string;
}

export interface CreateCustomRuleInput {
    rule_type: string;
    category: string;
    pattern: string;
    severity?: SecurityRuleSeverity;
    action?: string;
    description?: string;
}

export interface UpdateBuiltinRuleInput {
    severity?: SecurityRuleSeverity;
    title?: string;
    description?: string;
    enabled?: boolean;
}

export interface ApiKey {
    id: string;
    name: string;
    key: string;
    status: number;
    allowed_models: string[];
    allowed_channels: string[];
    quota_limit: number;
    quota_used: number;
    expires_at: string | null;
    created_at: string;
    updated_at: string;
}

export interface CreateApiKeyInput {
    name: string;
    quota_limit: number;
    allowed_models: string[];
    allowed_channels: string[];
    expires_at: string | null;
}

export interface UpdateApiKeyInput {
    id: string;
    status?: number;
    quota_limit?: number;
    expires_at?: string;
    clear_expires_at?: boolean;
}

export type RiskLevel = "clean" | "info" | "low" | "medium" | "high" | "critical";

export interface RequestLog {
    id: string;
    seq: number | null;
    api_key_name: string | null;
    channel_name: string | null;
    model: string;
    upstream_model: string | null;
    mode: string;
    status_code: number;
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    duration_ms: number;
    error_message: string | null;
    is_stream: boolean;
    is_retry: boolean;
    created_at: string;
    request_body: string | null;
    response_choices: string | null;
    risk_level: RiskLevel;
    risk_score: number;
    risk_summary: string | null;
    security_action: string;
    sanitized: boolean;
    blocked_reason: string | null;
    trace_id: string | null;
}

export interface RequestSecurityFinding {
    id: string;
    log_id: string;
    phase: string;
    category: string;
    rule_id: string;
    severity: RiskLevel;
    title: string;
    description: string | null;
    location: string | null;
    evidence_masked: string | null;
    action: string | null;
    created_at: string;
}

export interface GetLogsInput {
    keyword?: string;
    api_key_name?: string;
    channel_name?: string;
    model?: string;
    date_from?: string;
    date_to?: string;
    limit?: number;
    offset?: number;
    after_seq?: number;
}

export interface LogStats {
    date: string;
    count: number;
    total_tokens: number;
}

export interface ProtocolUsageStat {
    mode: string;
    request_count: number;
    total_tokens: number;
}

export interface UsageBucketStat {
    bucket_index: number;
    request_count: number;
}

export interface ModelUsageStat {
    name: string;
    request_count: number;
    total_tokens: number;
}

export interface ChannelUsageStat {
    id: string;
    name: string;
    channel_type: string;
    request_count: number;
}

export interface DashboardStatsInput {
    date_from?: string;
    date_to?: string;
}

export interface UsageStatsInput extends DashboardStatsInput {
    bucket_seconds?: number;
    bucket_count?: number;
}

export interface UsageStats {
    total_requests: number;
    total_tokens: number;
    failed_requests: number;
    protocols: ProtocolUsageStat[];
    series: UsageBucketStat[];
    models: ModelUsageStat[];
    channels: ChannelUsageStat[];
}

export interface DashboardStats {
    today_requests: number;
    today_total_tokens: number;
    active_channels: number;
    avg_latency_ms: number;
    total_channels: number;
    total_api_keys: number;
    total_requests: number;
    total_tokens: number;
    protocols: ProtocolUsageStat[];
}

export type UiTheme =
    | "light"
    | "system"
    | "dark"
    | "mist"
    | "ember"
    | "graphite"
    | "frost"
    | "sakura"
    | "mono"
    | "ocean"
    | "neon";

export interface Settings {
    server_port: number;
    server_host: string;
    ui_theme: UiTheme;
    ui_language: string;
    minimize_to_tray: boolean;
    close_to_tray: boolean;
    auto_start: boolean;
    retry_enabled: boolean;
    retry_times: number;
    default_key_quota: number;
    total_quota: number;
    quota_warning_threshold: number;
    security_enabled: boolean;
    security_mode: string;
    security_scan_unicode: boolean;
    security_scan_tools: boolean;
    security_scan_network: boolean;
    security_scan_response: boolean;
    security_redact_secrets: boolean;
    security_block_on_critical: boolean;
}

export interface ServerStatus {
    running: boolean;
    port: number;
    url: string;
}

export interface KnowledgeBase {
    id: string;
    name: string;
    description: string | null;
    status: number;
    doc_count: number;
    chunk_count: number;
    total_tokens: number;
    embedding_model: string | null;
    embedding_channel_id: string | null;
    mcp_enabled: number;
    chunk_size: number;
    chunk_overlap: number;
    excluded_dirs: string;
    excluded_files: string;
    included_files: string;
    embedding_dim: number;
    index_status: string;
    embedding_batch_size: number;
    created_at: string;
    updated_at: string;
}

export interface KbDocument {
    id: string;
    kb_id: string;
    filename: string;
    file_path: string | null;
    file_type: string;
    file_size: number;
    content_hash: string;
    chunk_count: number;
    token_count: number;
    status: string;
    error_message: string | null;
    source_type: string;
    source_url: string | null;
    source_path: string | null;
    doc_meta: string;
    created_at: string;
    updated_at: string;
}

export interface KbConversation {
    id: string;
    kb_id: string;
    role: string;
    content: string;
    sources: string | null;
    model: string | null;
    tokens_used: number;
    created_at: string;
}

export interface KbSource {
    id: string;
    kb_id: string;
    source_type: string;
    source_url: string | null;
    source_path: string | null;
    branch: string | null;
    status: string;
    file_count: number;
    error: string | null;
    created_at: string;
    updated_at: string;
}

export interface KbIndexMeta {
    kb_id: string;
    index_type: string;
    embedding_dim: number;
    chunk_count: number;
    index_path: string | null;
    built_at: string | null;
    status: string;
}

export interface ConversationMessage {
    role: string;
    content: string;
}

export interface KbSearchResult {
    chunk_id: string;
    doc_id: string;
    filename: string;
    content: string;
    score: number;
    metadata: Record<string, unknown>;
}

export interface KbRetrievalDetail {
    chunk_id: string;
    filename: string;
    score: number;
    vector_score: number | null;
    keyword_score: number | null;
    snippet: string;
    symbol_name: string | null;
    symbol_kind: string | null;
}

export interface KbRagAnswer {
    answer: string;
    sources: Array<{
        filename: string;
        score: number;
        snippet: string;
    }>;
    usage: {
        prompt_tokens: number;
        completion_tokens: number;
        total_tokens: number;
    } | null;
    retrieval_details: KbRetrievalDetail[] | null;
}

export interface KbTag {
    word: string;
    count: number;
}

export interface ServiceStatus {
    id: string;
    name: string;
    description: string;
    enabled: boolean;
    running: boolean;
    health: "healthy" | "degraded" | "unavailable";
    issues: Array<{
        code: string;
        message: string;
        retryable: boolean;
    }>;
    stats: Record<string, unknown>;
}
