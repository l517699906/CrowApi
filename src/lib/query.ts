export const queryKeys = {
    channels: ["channels"] as const,
    apiKeys: ["api-keys"] as const,
    logs: ["logs"] as const,
    logSecurityFindings: (logId: string) => ["logs", logId, "security-findings"] as const,
    dashboard: ["dashboard"] as const,
    dashboardStats: (dateFrom: string, dateTo: string) => (
        ["dashboard", dateFrom, dateTo] as const
    ),
    usageStats: (period: string, dateFrom = "*", dateTo = "*") => (
        ["usage-stats", period, dateFrom, dateTo] as const
    ),
    settings: ["settings"] as const,
    masterKeyStatus: ["master-key-status"] as const,
    securityBuiltinRules: ["security-rules", "builtin"] as const,
    securityCustomRules: ["security-rules", "custom"] as const,
    serverStatus: ["server-status"] as const,
    serviceStatuses: ["service-statuses"] as const,
    backgroundTasksRoot: ["background-tasks"] as const,
    backgroundTasks: (
        domain = "*",
        resourceType = "*",
        resourceId = "*",
        status = "*",
        limit: number | "*" = "*",
    ) => ["background-tasks", domain, resourceType, resourceId, status, limit] as const,
    knowledgeBases: ["knowledge-bases"] as const,
    knowledgeBase: (id: string) => ["knowledge-bases", id] as const,
    kbDocuments: (id: string) => ["knowledge-bases", id, "documents"] as const,
    kbSources: (id: string) => ["knowledge-bases", id, "sources"] as const,
    kbTags: (id: string) => ["knowledge-bases", id, "tags"] as const,
    kbIndex: (id: string) => ["knowledge-bases", id, "index"] as const,
    kbStats: (id: string) => ["knowledge-bases", id, "stats"] as const,
    wikiProjects: ["wiki-projects"] as const,
    wikiProject: (id: string) => ["wiki-projects", id] as const,
    wikiPages: (id: string) => ["wiki-projects", id, "pages"] as const,
    wikiPage: (id: string, path: string) => ["wiki-projects", id, "pages", path] as const,
    wikiSources: (id: string) => ["wiki-projects", id, "sources"] as const,
    wikiTags: (id: string) => ["wiki-projects", id, "tags"] as const,
    wikiStats: (id: string) => ["wiki-projects", id, "stats"] as const,
    wikiGraph: (id: string) => ["wiki-projects", id, "graph"] as const,
    wikiSearch: (id: string, query: string, offset = 0) => (
        ["wiki-projects", id, "search", query, offset] as const
    ),
};

export interface AppError {
    code: string;
    message: string;
    trace_id?: string;
    retryable: boolean;
    details?: unknown;
}

export class AppErrorException extends Error {
    readonly code: string;
    readonly retryable: boolean;
    readonly trace_id?: string;
    readonly details?: unknown;

    constructor(error: AppError) {
        super(error.message);
        this.name = "AppErrorException";
        this.code = error.code;
        this.retryable = error.retryable;
        this.trace_id = error.trace_id;
        this.details = error.details;
    }
}

const DEFAULT_ERROR_MESSAGE = "操作失败，请稍后重试";
const TAURI_UNAVAILABLE_MESSAGE = "此功能仅可在 CrowAPI 桌面应用中使用";

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isTauriUnavailable(message: string) {
    return message.includes("reading 'invoke'") || message.includes("__TAURI_INTERNALS__");
}

function fromMessage(message: string, code = "UNKNOWN"): AppError {
    if (isTauriUnavailable(message)) {
        return {
            code: "TAURI_UNAVAILABLE",
            message: TAURI_UNAVAILABLE_MESSAGE,
            retryable: false,
        };
    }

    return {
        code,
        message: message.trim() || DEFAULT_ERROR_MESSAGE,
        retryable: false,
    };
}

function fromRecord(error: Record<string, unknown>): AppError {
    const message = typeof error.message === "string" ? error.message : DEFAULT_ERROR_MESSAGE;
    const normalized = fromMessage(
        message,
        typeof error.code === "string" && error.code.trim() ? error.code : "UNKNOWN",
    );

    const traceId = typeof error.trace_id === "string"
        ? error.trace_id
        : typeof error.traceId === "string"
            ? error.traceId
            : undefined;

    return {
        ...normalized,
        retryable: typeof error.retryable === "boolean" ? error.retryable : normalized.retryable,
        ...(traceId ? { trace_id: traceId } : {}),
        ...("details" in error ? { details: error.details } : {}),
    };
}

function parseStructuredError(value: string): Record<string, unknown> | undefined {
    if (!value.trimStart().startsWith("{")) return undefined;

    try {
        const parsed: unknown = JSON.parse(value);
        return isRecord(parsed) ? parsed : undefined;
    } catch {
        return undefined;
    }
}

export function normalizeAppError(error: unknown): AppError {
    if (error instanceof AppErrorException) {
        return {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
            ...(error.trace_id ? { trace_id: error.trace_id } : {}),
            ...(error.details !== undefined ? { details: error.details } : {}),
        };
    }
    if (error instanceof Error) return fromMessage(error.message);
    if (typeof error === "string") {
        const structured = parseStructuredError(error);
        return structured ? fromRecord(structured) : fromMessage(error);
    }
    if (isRecord(error)) return fromRecord(error);
    return fromMessage(DEFAULT_ERROR_MESSAGE);
}

export function errorMessage(error: unknown): string {
    return normalizeAppError(error).message;
}

export function nextPollingInterval(
    baseIntervalMs: number,
    failureCount: number,
    maximumIntervalMs = 60_000,
): number {
    const base = Math.max(1_000, baseIntervalMs);
    const maximum = Math.max(base, maximumIntervalMs);
    const exponent = Math.min(Math.max(0, failureCount), 4);
    return Math.min(base * (2 ** exponent), maximum);
}
