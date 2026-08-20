export const queryKeys = {
    channels: ["channels"] as const,
    apiKeys: ["api-keys"] as const,
    logs: ["logs"] as const,
    logSecurityFindings: (logId: string) => ["logs", logId, "security-findings"] as const,
    dashboard: ["dashboard"] as const,
    usageStats: (period: string) => ["usage-stats", period] as const,
    settings: ["settings"] as const,
    securityBuiltinRules: ["security-rules", "builtin"] as const,
    securityCustomRules: ["security-rules", "custom"] as const,
    serverStatus: ["server-status"] as const,
};

export interface AppError {
    code: string;
    message: string;
    trace_id?: string;
    retryable: boolean;
    details?: unknown;
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
