export const queryKeys = {
    channels: ["channels"] as const,
    apiKeys: ["api-keys"] as const,
    logs: ["logs"] as const,
    logSecurityFindings: (logId: string) => ["logs", logId, "security-findings"] as const,
    dashboard: ["dashboard"] as const,
    usageStats: (period: string) => ["usage-stats", period] as const,
    settings: ["settings"] as const,
    serverStatus: ["server-status"] as const,
};

export function errorMessage(error: unknown): string {
    if (error instanceof Error) {
        return error.message;
    }
    if (typeof error === "string") {
        return error;
    }
    return "操作失败，请稍后重试";
}
