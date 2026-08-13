const compactNumberFormatter = new Intl.NumberFormat("zh-CN", {
    notation: "compact",
    maximumFractionDigits: 1,
});

const numberFormatter = new Intl.NumberFormat("zh-CN");

export function formatCompactNumber(value: number): string {
    return compactNumberFormatter.format(value);
}

export function formatNumber(value: number): string {
    return numberFormatter.format(value);
}

export function formatDuration(value: number): string {
    return value >= 1000 ? `${(value / 1000).toFixed(2)} s` : `${value} ms`;
}

export function formatDateTime(value: string): string {
    return new Intl.DateTimeFormat("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
    }).format(new Date(value));
}

export function formatQuota(value: number): string {
    return value === 0 ? "不限" : formatCompactNumber(value);
}

export function maskSecret(value: string): string {
    if (value.length <= 16) {
        return value;
    }

    return `${value.slice(0, 12)}${"•".repeat(8)}${value.slice(-5)}`;
}
