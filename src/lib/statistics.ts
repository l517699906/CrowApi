import type { DashboardStatsInput, UsageBucketStat, UsageStatsInput } from "../types";

export function localDayStatsInput(): DashboardStatsInput {
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    const end = new Date(start);
    end.setDate(end.getDate() + 1);
    return { date_from: start.toISOString(), date_to: end.toISOString() };
}

export function rollingUsageStatsInput(bucketCount: number, bucketSeconds: number): UsageStatsInput {
    const end = new Date();
    return {
        date_from: new Date(end.getTime() - bucketCount * bucketSeconds * 1_000).toISOString(),
        date_to: end.toISOString(),
        bucket_seconds: bucketSeconds,
        bucket_count: bucketCount,
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
