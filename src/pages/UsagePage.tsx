import { useMemo, useState } from "react";
import { Coins, Database, Gauge, Layers3, TriangleAlert } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { formatCompactNumber, formatNumber } from "../lib/format";
import { channelApi, logApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import { PageTitle, ProviderMark, SegmentedControl } from "../components/ui";
import type { RequestLog } from "../types";

type UsagePeriod = "24h" | "7d" | "30d";

const periodOptions: ReadonlyArray<{ value: UsagePeriod; label: string }> = [
    { value: "24h", label: "24 小时" },
    { value: "7d", label: "7 天" },
    { value: "30d", label: "30 天" },
];

const periodDays: Record<UsagePeriod, number> = {
    "24h": 1,
    "7d": 7,
    "30d": 30,
};

const modelColors = ["var(--accent)", "var(--data-blue)", "var(--warning)", "var(--coral)"];

function periodStart(period: UsagePeriod): Date {
    return new Date(Date.now() - periodDays[period] * 86_400_000);
}

function buildSeries(logs: RequestLog[], period: UsagePeriod): number[] {
    const bucketCount = period === "24h" ? 24 : periodDays[period];
    const bucketMs = period === "24h" ? 3_600_000 : 86_400_000;
    const start = periodStart(period).getTime();
    const values = Array.from({ length: bucketCount }, () => 0);

    logs.forEach((log) => {
        const index = Math.floor((new Date(log.created_at).getTime() - start) / bucketMs);
        if (index >= 0 && index < values.length) {
            values[index] += 1;
        }
    });
    return values;
}

export function UsagePage() {
    const [period, setPeriod] = useState<UsagePeriod>("24h");
    const start = periodStart(period).toISOString();
    const { data: logs = [], isPending, error } = useQuery({
        queryKey: [...queryKeys.logs, "usage", period],
        queryFn: () => logApi.getAll({ date_from: start, limit: 5_000 }),
    });
    const { data: channels = [] } = useQuery({
        queryKey: queryKeys.channels,
        queryFn: channelApi.getAll,
    });
    const series = useMemo(() => buildSeries(logs, period), [logs, period]);
    const chartMax = Math.max(...series, 1);
    const totalRequests = logs.length;
    const totalTokens = logs.reduce((sum, log) => sum + log.total_tokens, 0);
    const averageTokens = totalRequests > 0 ? Math.round(totalTokens / totalRequests) : 0;
    const failedRequests = logs.filter((log) => log.status_code >= 400).length;
    const successRate = totalRequests > 0 ? ((totalRequests - failedRequests) / totalRequests) * 100 : 100;
    const modelUsage = useMemo(() => {
        const usage = new Map<string, { name: string; requests: number; tokens: number }>();
        logs.forEach((log) => {
            const current = usage.get(log.model) ?? { name: log.model, requests: 0, tokens: 0 };
            current.requests += 1;
            current.tokens += log.total_tokens;
            usage.set(log.model, current);
        });
        return [...usage.values()].sort((left, right) => right.tokens - left.tokens);
    }, [logs]);
    const channelUsage = useMemo(() => {
        const usage = new Map<string, number>();
        logs.forEach((log) => {
            const name = log.channel_name ?? "未分配渠道";
            usage.set(name, (usage.get(name) ?? 0) + 1);
        });
        return [...usage.entries()]
            .map(([name, requests]) => {
                const channel = channels.find((item) => item.name === name);
                return { id: channel?.id ?? name, name, type: channel?.type ?? "custom", requests };
            })
            .sort((left, right) => right.requests - left.requests);
    }, [channels, logs]);

    return (
        <div className="page-enter">
            <PageTitle
                title="用量"
                meta="请求与 Token 消耗"
                action={(
                    <SegmentedControl
                        value={period}
                        options={periodOptions}
                        onChange={setPeriod}
                        label="用量统计周期"
                    />
                )}
            />

            <section className="usage-summary-band">
                <div className="usage-total">
                    <p>总 Token</p>
                    <strong>{formatCompactNumber(totalTokens)}</strong>
                    <span>{periodOptions.find((option) => option.value === period)?.label}</span>
                </div>
                <div className="usage-band-divider" />
                <div className="usage-band-stat">
                    <span><Layers3 size={16} />请求</span>
                    <strong>{formatNumber(totalRequests)}</strong>
                </div>
                <div className="usage-band-stat">
                    <span><Gauge size={16} />平均 Token</span>
                    <strong>{formatNumber(averageTokens)}</strong>
                </div>
                <div className="usage-band-stat">
                    <span><Database size={16} />成功率</span>
                    <strong>{successRate.toFixed(1)}%</strong>
                </div>
            </section>

            <section className="panel mt-4">
                <div className="panel-header">
                    <div>
                        <h2>请求分布</h2>
                        <p>{periodOptions.find((option) => option.value === period)?.label}</p>
                    </div>
                    <span className="text-xs font-medium text-accent">{isPending ? "读取中" : `${totalRequests} 次请求`}</span>
                </div>
                <div className={`usage-chart usage-chart-${period}`} aria-label="请求用量柱状图">
                    {series.map((value, index) => (
                        <div className="usage-chart-column" key={`${value}-${index}`}>
                            <span
                                className="usage-chart-bar"
                                style={{ height: `${Math.max((value / chartMax) * 100, 5)}%` }}
                                title={`${value} 次请求`}
                            />
                        </div>
                    ))}
                </div>
                <div className="chart-axis">
                    <span>{period === "24h" ? "24 小时前" : "开始"}</span>
                    <span>25%</span><span>50%</span><span>75%</span>
                    <span>{period === "24h" ? "现在" : "今天"}</span>
                </div>
            </section>

            <section className="usage-detail-grid">
                <article className="panel">
                    <div className="panel-header">
                        <div><h2>模型消耗</h2><p>按 Token 排序</p></div>
                        <Coins size={18} className="text-muted" />
                    </div>
                    <div className="model-usage-list">
                        {modelUsage.map((model, index) => {
                            const percentage = modelUsage[0]?.tokens ? (model.tokens / modelUsage[0].tokens) * 100 : 0;
                            return (
                                <div className="usage-list-row" key={model.name}>
                                    <div className="flex items-center justify-between gap-4">
                                        <span className="model-name">{model.name}</span>
                                        <span className="font-mono text-xs font-semibold text-ink">{formatCompactNumber(model.tokens)}</span>
                                    </div>
                                    <div className="usage-progress"><span style={{ width: `${percentage}%`, background: modelColors[index % modelColors.length] }} /></div>
                                    <div className="flex items-center justify-between text-[11px] text-subtle">
                                        <span>{formatNumber(model.requests)} 次</span>
                                        <span>{totalTokens > 0 ? Math.round((model.tokens / totalTokens) * 100) : 0}%</span>
                                    </div>
                                </div>
                            );
                        })}
                        {modelUsage.length === 0 ? <div className="empty-state"><Coins size={22} /><strong>暂无模型用量</strong></div> : null}
                    </div>
                </article>

                <article className="panel">
                    <div className="panel-header">
                        <div><h2>渠道占比</h2><p>按请求数排序</p></div>
                        <span className="text-xs text-muted">失败 {failedRequests} 次</span>
                    </div>
                    <div className="channel-usage-list">
                        {channelUsage.map((channel, index) => (
                            <div className="channel-usage-row" key={channel.id}>
                                <span className="font-mono text-xs text-subtle">{String(index + 1).padStart(2, "0")}</span>
                                <ProviderMark type={channel.type} size="sm" />
                                <div className="min-w-0 flex-1">
                                    <p className="truncate text-sm font-medium text-ink">{channel.name}</p>
                                    <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-soft">
                                        <span className="block h-full rounded-full bg-data-blue" style={{ width: `${channelUsage[0]?.requests ? (channel.requests / channelUsage[0].requests) * 100 : 0}%` }} />
                                    </div>
                                </div>
                                <strong className="font-mono text-xs text-ink">{formatNumber(channel.requests)}</strong>
                            </div>
                        ))}
                        {channelUsage.length === 0 ? <div className="empty-state"><Database size={22} /><strong>暂无渠道用量</strong></div> : null}
                    </div>
                </article>
            </section>
            {error ? <p className="form-error mt-4" role="alert"><TriangleAlert size={14} className="inline" /> {errorMessage(error)}</p> : null}
        </div>
    );
}
