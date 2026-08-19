import { useMemo, useState } from "react";
import { Coins, Database, Gauge, Layers3, TriangleAlert } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { formatCompactNumber, formatNumber } from "../lib/format";
import { statsApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import { getProtocolMeta, protocolTotal } from "../lib/protocol";
import { materializeUsageSeries, rollingUsageStatsInput } from "../lib/statistics";
import { PageTitle, ProviderMark, SegmentedControl } from "../components/ui";

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

function usageStatsInput(period: UsagePeriod) {
    const bucketCount = period === "24h" ? 24 : periodDays[period];
    const bucketSeconds = period === "24h" ? 3_600 : 86_400;
    return rollingUsageStatsInput(bucketCount, bucketSeconds);
}

export function UsagePage() {
    const [period, setPeriod] = useState<UsagePeriod>("24h");
    const { data: usageStats, isPending, error } = useQuery({
        queryKey: queryKeys.usageStats(period),
        queryFn: () => statsApi.getUsage(usageStatsInput(period)),
        refetchInterval: 5_000,
    });
    const series = useMemo(
        () => materializeUsageSeries(usageStats?.series ?? [], period === "24h" ? 24 : periodDays[period]),
        [period, usageStats?.series],
    );
    const chartMax = Math.max(...series, 1);
    const totalRequests = usageStats?.total_requests ?? 0;
    const totalTokens = usageStats?.total_tokens ?? 0;
    const averageTokens = totalRequests > 0 ? Math.round(totalTokens / totalRequests) : 0;
    const failedRequests = usageStats?.failed_requests ?? 0;
    const successRate = totalRequests > 0 ? ((totalRequests - failedRequests) / totalRequests) * 100 : 100;
    const protocolStats = usageStats?.protocols ?? [];
    const totalProtocolRequests = protocolTotal(protocolStats);
    const modelUsage = usageStats?.models ?? [];
    const channelUsage = usageStats?.channels ?? [];

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
                                style={{
                                    height: value === 0 ? 0 : `${Math.max((value / chartMax) * 100, 5)}%`,
                                    minHeight: value === 0 ? 0 : undefined,
                                }}
                                title={`${value} 次请求`}
                                role="img"
                                aria-label={`第 ${index + 1} 个时间段，${value} 次请求`}
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
                            const percentage = modelUsage[0]?.total_tokens ? (model.total_tokens / modelUsage[0].total_tokens) * 100 : 0;
                            return (
                                <div className="usage-list-row" key={model.name}>
                                    <div className="flex items-center justify-between gap-4">
                                        <span className="model-name">{model.name}</span>
                                        <span className="font-mono text-xs font-semibold text-ink">{formatCompactNumber(model.total_tokens)}</span>
                                    </div>
                                    <div className="usage-progress"><span style={{ width: `${percentage}%`, background: modelColors[index % modelColors.length] }} /></div>
                                    <div className="flex items-center justify-between text-[11px] text-subtle">
                                        <span>{formatNumber(model.request_count)} 次</span>
                                        <span>{totalTokens > 0 ? Math.round((model.total_tokens / totalTokens) * 100) : 0}%</span>
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
                                <ProviderMark type={channel.channel_type} size="sm" />
                                <div className="min-w-0 flex-1">
                                    <p className="truncate text-sm font-medium text-ink">{channel.name}</p>
                                    <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-soft">
                                        <span className="block h-full rounded-full bg-data-blue" style={{ width: `${channelUsage[0]?.request_count ? (channel.request_count / channelUsage[0].request_count) * 100 : 0}%` }} />
                                    </div>
                                </div>
                                <strong className="font-mono text-xs text-ink">{formatNumber(channel.request_count)}</strong>
                            </div>
                        ))}
                        {channelUsage.length === 0 ? <div className="empty-state"><Database size={22} /><strong>暂无渠道用量</strong></div> : null}
                    </div>
                </article>

                <article className="panel protocol-usage-panel">
                    <div className="panel-header">
                        <div><h2>协议维度</h2><p>请求数与 Token 消耗</p></div>
                        <span className="text-xs text-muted">{formatNumber(totalProtocolRequests)} 次</span>
                    </div>
                    <div className="protocol-usage-list">
                        {protocolStats.map((item) => {
                            const protocol = getProtocolMeta(item.mode);
                            const percentage = totalProtocolRequests > 0 ? (item.request_count / totalProtocolRequests) * 100 : 0;
                            return (
                                <div className="protocol-usage-row" key={item.mode}>
                                    <div className="protocol-usage-heading">
                                        <span className={`protocol-badge protocol-${protocol.tone}`}>{protocol.label}</span>
                                        <span>{percentage.toFixed(1)}%</span>
                                    </div>
                                    <div className="usage-progress">
                                        <span className={`protocol-fill protocol-fill-${protocol.tone}`} style={{ width: `${percentage}%` }} />
                                    </div>
                                    <div className="protocol-usage-values">
                                        <span>{formatNumber(item.request_count)} 次请求</span>
                                        <strong>{formatCompactNumber(item.total_tokens)} Token</strong>
                                    </div>
                                </div>
                            );
                        })}
                        {protocolStats.length === 0 ? <div className="empty-state"><Layers3 size={22} /><strong>暂无协议用量</strong></div> : null}
                    </div>
                </article>
            </section>
            {error ? <p className="form-error mt-4" role="alert"><TriangleAlert size={14} className="inline" /> {errorMessage(error)}</p> : null}
        </div>
    );
}
