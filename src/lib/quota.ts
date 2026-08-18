export const MAX_QUOTA = Number.MAX_SAFE_INTEGER;

export function normalizeQuota(value: number | string): number {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) {
        return 0;
    }
    return Math.min(MAX_QUOTA, Math.max(0, Math.trunc(parsed)));
}
