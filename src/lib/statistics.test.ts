import { describe, expect, it } from "vitest";
import {
    localDayStatsInput,
    normalizeDashboardStats,
    normalizeUsageStats,
    rollingUsageCacheRange,
    rollingUsageStatsInput,
} from "./statistics";

describe("statistics ranges", () => {
    it("uses a half-open local-day range", () => {
        const range = localDayStatsInput(new Date("2026-08-21T12:30:00+08:00"));
        expect(range).toEqual({
            date_from: "2026-08-20T16:00:00.000Z",
            date_to: "2026-08-21T16:00:00.000Z",
        });
    });

    it("keeps live rolling inputs exact while cache keys align to a bucket", () => {
        const now = new Date("2026-08-21T04:37:42.123Z");
        const input = rollingUsageStatsInput(24, 3_600, now);
        const cacheRange = rollingUsageCacheRange(24, 3_600, now);

        expect(input.date_to).toBe("2026-08-21T04:37:42.123Z");
        expect(cacheRange).toEqual({
            dateFrom: "2026-08-20T04:00:00.000Z",
            dateTo: "2026-08-21T04:00:00.000Z",
        });
    });
});

describe("statistics response contracts", () => {
    it("normalizes a compatible dashboard response and optional lists", () => {
        expect(normalizeDashboardStats({
            today_requests: 3,
            today_total_tokens: 9,
            avg_latency_ms: 12.5,
            total_requests: 8,
            total_tokens: 20,
            protocols: null,
        })).toMatchObject({
            today_requests: 3,
            active_channels: 0,
            total_channels: 0,
            protocols: [],
        });
    });

    it("rejects malformed dashboard and usage payloads before rendering", () => {
        expect(() => normalizeDashboardStats({ today_requests: "3" })).toThrow("today_requests");
        expect(() => normalizeUsageStats({
            total_requests: 1,
            total_tokens: 1,
            failed_requests: 0,
            series: [{ bucket_index: 0 }],
        })).toThrow("request_count");
    });
});
