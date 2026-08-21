import type {
    ChannelUsageStat,
    DashboardStats,
    DashboardStatsInput,
    ModelUsageStat,
    ProtocolUsageStat,
    UsageBucketStat,
    UsageStats,
    UsageStatsInput,
} from "../types";
import { AppErrorException } from "./query";

function asRecord(value: unknown, label: string): Record<string, unknown> {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
        throw new AppErrorException({
            code: `${label.toUpperCase()}_RESPONSE_INVALID`,
            message: `${label}返回的数据格式无效`,
            retryable: true,
        });
    }
    return value as Record<string, unknown>;
}

function requiredNumber(record: Record<string, unknown>, key: string, label: string): number {
    const value = record[key];
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new AppErrorException({
            code: `${label.toUpperCase()}_RESPONSE_INVALID`,
            message: `${label}缺少有效字段 ${key}`,
            retryable: true,
            details: { field: key },
        });
    }
    return value;
}

function optionalNumber(record: Record<string, unknown>, key: string, fallback = 0): number {
    const value = record[key];
    return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function stringField(record: Record<string, unknown>, key: string, label: string): string {
    const value = record[key];
    if (typeof value !== "string") {
        throw new AppErrorException({
            code: `${label.toUpperCase()}_RESPONSE_INVALID`,
            message: `${label}缺少有效字段 ${key}`,
            retryable: true,
            details: { field: key },
        });
    }
    return value;
}

function normalizeProtocols(value: unknown, label: string): ProtocolUsageStat[] {
    if (value == null) return [];
    if (!Array.isArray(value)) {
        throw new AppErrorException({
            code: `${label.toUpperCase()}_RESPONSE_INVALID`,
            message: `${label}的 protocols 字段格式无效`,
            retryable: true,
            details: { field: "protocols" },
        });
    }
    return value.map((item) => {
        const record = asRecord(item, label);
        return {
            mode: stringField(record, "mode", label),
            request_count: requiredNumber(record, "request_count", label),
            total_tokens: requiredNumber(record, "total_tokens", label),
        };
    });
}

function normalizeUsageSeries(value: unknown, label: string): UsageBucketStat[] {
    return (Array.isArray(value) ? value : []).map((item) => {
        const record = asRecord(item, label);
        return {
            bucket_index: requiredNumber(record, "bucket_index", label),
            request_count: requiredNumber(record, "request_count", label),
        };
    });
}

function normalizeModels(value: unknown, label: string): ModelUsageStat[] {
    return (Array.isArray(value) ? value : []).map((item) => {
        const record = asRecord(item, label);
        return {
            name: stringField(record, "name", label),
            request_count: requiredNumber(record, "request_count", label),
            total_tokens: requiredNumber(record, "total_tokens", label),
        };
    });
}

function normalizeChannels(value: unknown, label: string): ChannelUsageStat[] {
    return (Array.isArray(value) ? value : []).map((item) => {
        const record = asRecord(item, label);
        return {
            id: stringField(record, "id", label),
            name: stringField(record, "name", label),
            channel_type: stringField(record, "channel_type", label),
            request_count: requiredNumber(record, "request_count", label),
        };
    });
}

export function normalizeDashboardStats(value: unknown): DashboardStats {
    const record = asRecord(value, "dashboard");
    return {
        today_requests: requiredNumber(record, "today_requests", "dashboard"),
        today_total_tokens: requiredNumber(record, "today_total_tokens", "dashboard"),
        active_channels: optionalNumber(record, "active_channels"),
        avg_latency_ms: requiredNumber(record, "avg_latency_ms", "dashboard"),
        total_channels: optionalNumber(record, "total_channels"),
        total_api_keys: optionalNumber(record, "total_api_keys"),
        total_requests: requiredNumber(record, "total_requests", "dashboard"),
        total_tokens: requiredNumber(record, "total_tokens", "dashboard"),
        protocols: normalizeProtocols(record.protocols, "dashboard"),
    };
}

export function normalizeUsageStats(value: unknown): UsageStats {
    const record = asRecord(value, "usage");
    return {
        total_requests: requiredNumber(record, "total_requests", "usage"),
        total_tokens: requiredNumber(record, "total_tokens", "usage"),
        failed_requests: requiredNumber(record, "failed_requests", "usage"),
        protocols: normalizeProtocols(record.protocols, "usage"),
        series: normalizeUsageSeries(record.series, "usage"),
        models: normalizeModels(record.models, "usage"),
        channels: normalizeChannels(record.channels, "usage"),
    };
}

export function localDayStatsInput(now = new Date()): DashboardStatsInput {
    const start = new Date(now);
    start.setHours(0, 0, 0, 0);
    const end = new Date(start);
    end.setDate(end.getDate() + 1);
    return { date_from: start.toISOString(), date_to: end.toISOString() };
}

export function rollingUsageStatsInput(
    bucketCount: number,
    bucketSeconds: number,
    now = new Date(),
): UsageStatsInput {
    const end = new Date(now);
    return {
        date_from: new Date(end.getTime() - bucketCount * bucketSeconds * 1_000).toISOString(),
        date_to: end.toISOString(),
        bucket_seconds: bucketSeconds,
        bucket_count: bucketCount,
    };
}

export function rollingUsageCacheRange(
    bucketCount: number,
    bucketSeconds: number,
    now = new Date(),
): { dateFrom: string; dateTo: string } {
    const bucketMilliseconds = bucketSeconds * 1_000;
    const alignedEnd = new Date(
        Math.floor(now.getTime() / bucketMilliseconds) * bucketMilliseconds,
    );
    return {
        dateFrom: new Date(
            alignedEnd.getTime() - bucketCount * bucketMilliseconds,
        ).toISOString(),
        dateTo: alignedEnd.toISOString(),
    };
}

export function materializeUsageSeries(buckets: UsageBucketStat[], bucketCount: number): number[] {
    const values = Array.from({ length: bucketCount }, () => 0);
    buckets.forEach((bucket) => {
        if (bucket.bucket_index >= 0 && bucket.bucket_index < values.length) {
            values[bucket.bucket_index] = bucket.request_count;
        }
    });
    return values;
}
