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

export function errorMessage(error: unknown): string {
    if (error instanceof Error) {
        if (error.message.includes("reading 'invoke'") || error.message.includes("__TAURI_INTERNALS__")) {
            return "此功能仅可在 CrowAPI 桌面应用中使用";
        }
        return error.message;
    }
    if (typeof error === "string") {
        if (error.includes("reading 'invoke'") || error.includes("__TAURI_INTERNALS__")) {
            return "此功能仅可在 CrowAPI 桌面应用中使用";
        }
        return error;
    }
    return "操作失败，请稍后重试";
}
