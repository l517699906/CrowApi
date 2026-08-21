import type { KbIndexMeta } from "../types";
import { AppErrorException } from "./query";

function invalid(field?: string): never {
    throw new AppErrorException({
        code: "KB_INDEX_RESPONSE_INVALID",
        message: field
            ? `知识库索引状态缺少有效字段 ${field}`
            : "知识库索引状态返回的数据格式无效",
        retryable: true,
        ...(field ? { details: { field } } : {}),
    });
}

function record(value: unknown): Record<string, unknown> {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
        return invalid();
    }
    return value as Record<string, unknown>;
}

function requiredString(value: unknown, field: string): string {
    return typeof value === "string" ? value : invalid(field);
}

function requiredInteger(value: unknown, field: string): number {
    return typeof value === "number" && Number.isSafeInteger(value) ? value : invalid(field);
}

function nullableString(value: unknown, field: string, fallback: string | null = null): string | null {
    if (value === undefined) return fallback;
    return value === null || typeof value === "string" ? value : invalid(field);
}

function compatibleInteger(value: unknown, field: string, fallback: number): number {
    if (value === undefined) return fallback;
    return requiredInteger(value, field);
}

/**
 * Validate and normalize the index metadata boundary. Older desktop builds did
 * not return integrity fields, so those fields receive their pre-migration
 * zero/null values while current builds preserve the complete manifest.
 */
export function normalizeKbIndexMeta(value: unknown): KbIndexMeta | null {
    if (value === null || value === undefined) return null;
    const item = record(value);
    return {
        kb_id: requiredString(item.kb_id, "kb_id"),
        index_type: requiredString(item.index_type, "index_type"),
        embedding_dim: requiredInteger(item.embedding_dim, "embedding_dim"),
        chunk_count: requiredInteger(item.chunk_count, "chunk_count"),
        index_path: nullableString(item.index_path, "index_path"),
        built_at: nullableString(item.built_at, "built_at"),
        status: requiredString(item.status, "status"),
        indexed_revision: requiredInteger(item.indexed_revision, "indexed_revision"),
        format_version: compatibleInteger(item.format_version, "format_version", 0),
        config_revision: compatibleInteger(item.config_revision, "config_revision", 0),
        content_fingerprint: nullableString(item.content_fingerprint, "content_fingerprint"),
        index_checksum: nullableString(item.index_checksum, "index_checksum"),
    };
}
