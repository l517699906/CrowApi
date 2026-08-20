import type { ProtocolUsageStat } from "../types";

export type ProtocolTone = "chat" | "anthropic" | "responses" | "embedding" | "other";

export interface ProtocolMeta {
    label: string;
    shortLabel: string;
    tone: ProtocolTone;
}

const PROTOCOLS: Record<string, ProtocolMeta> = {
    chat: { label: "OpenAI Chat", shortLabel: "Chat", tone: "chat" },
    completion: { label: "OpenAI Completions", shortLabel: "Completions", tone: "chat" },
    anthropic: { label: "Anthropic Messages", shortLabel: "Messages", tone: "anthropic" },
    responses: { label: "OpenAI Responses", shortLabel: "Responses", tone: "responses" },
    embedding: { label: "OpenAI Embeddings", shortLabel: "Embeddings", tone: "embedding" },
};

const OTHER_PROTOCOL: ProtocolMeta = {
    label: "其他请求",
    shortLabel: "其他",
    tone: "other",
};

export function getProtocolMeta(mode: string): ProtocolMeta {
    return PROTOCOLS[mode.toLowerCase()] ?? OTHER_PROTOCOL;
}

export function protocolTotal(stats: ProtocolUsageStat[]): number {
    return stats.reduce((sum, item) => sum + item.request_count, 0);
}

export function protocolGradient(stats: ProtocolUsageStat[]): string {
    const total = protocolTotal(stats);
    if (total <= 0) {
        return "var(--soft)";
    }

    let cursor = 0;
    const stops = stats.map((item) => {
        const start = cursor;
        cursor += (item.request_count / total) * 100;
        const color = `var(--protocol-${getProtocolMeta(item.mode).tone})`;
        return `${color} ${start.toFixed(2)}% ${cursor.toFixed(2)}%`;
    });
    return `conic-gradient(${stops.join(", ")})`;
}
