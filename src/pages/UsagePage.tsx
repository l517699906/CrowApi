import { useMemo, useState } from "react";
import { ArrowDownRight, ArrowUpRight, Coins, Database, Gauge, Layers3 } from "lucide-react";
import { modelUsage, requestSeries } from "../data/mockData";
import { formatCompactNumber, formatNumber } from "../lib/format";
import { useGatewayStore } from "../store/gatewayStore";
import { PageTitle, ProviderMark, SegmentedControl } from "../components/ui";

type UsagePeriod = keyof typeof requestSeries;

const periodOptions: ReadonlyArray<{ value: UsagePeriod; label: string }> = [
    { value: "24h", label: "24 小时" },
    { value: "7d", label: "7 天" },
    { value: "30d", label: "30 天" },
];

const periodMultipliers: Record<UsagePeriod, number> = {
    "24h": 1,
    "7d": 6.8,
    "30d": 28.4,
};

export function UsagePage() {
    const [period, setPeriod] = useState<UsagePeriod>("24h");
    const channels = useGatewayStore((state) => state.channels);
    const multiplier = periodMultipliers[period];
    const series = requestSeries[period];
    const chartMax = Math.max(...series);
    const totalRequests = Math.round(2841 * multiplier);
    const totalTokens = Math.round(2_946_000 * multiplier);
    const channelUsage = useMemo(() => channels.map((channel, index) => ({
        ...channel,
        requests: Math.max(Math.round(totalRequests * [0.42, 0.28, 0.21, 0.09][index % 4]), 0),
    })), [channels, totalRequests]);

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
                    <span><ArrowUpRight size={14} /> 8.4%</span>
                </div>
                <div className="usage-band-divider" />
                <div className="usage-band-stat">
                    <span><Layers3 size={16} />请求</span>
                    <strong>{formatNumber(totalRequests)}</strong>
                </div>
                <div className="usage-band-stat">
                    <span><Gauge size={16} />平均 Token</span>
                    <strong>{formatNumber(Math.round(totalTokens / totalRequests))}</strong>
                </div>
                <div className="usage-band-stat">
                    <span><Database size={16} />缓存命中</span>
                    <strong>38.6%</strong>
                </div>
            </section>

            <section className="panel mt-4">
                <div className="panel-header">
                    <div>
                        <h2>请求分布</h2>
                        <p>{periodOptions.find((option) => option.value === period)?.label}</p>
                    </div>
                    <span className="flex items-center gap-1 text-xs font-medium text-accent"><ArrowUpRight size={14} />峰值稳定</span>
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
                    <span>{period === "24h" ? "00:00" : "开始"}</span>
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
                        {modelUsage.map((model) => {
                            const tokens = Math.round(model.tokens * multiplier);
                            const percentage = (model.tokens / modelUsage[0].tokens) * 100;
                            return (
                                <div className="usage-list-row" key={model.name}>
                                    <div className="flex items-center justify-between gap-4">
                                        <span className="model-name">{model.name}</span>
                                        <span className="font-mono text-xs font-semibold text-ink">{formatCompactNumber(tokens)}</span>
                                    </div>
                                    <div className="usage-progress"><span style={{ width: `${percentage}%`, background: model.color }} /></div>
                                    <div className="flex items-center justify-between text-[11px] text-subtle">
                                        <span>{formatNumber(Math.round(model.requests * multiplier))} 次</span>
                                        <span>{Math.round((model.tokens / 2_946_000) * 100)}%</span>
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                </article>

                <article className="panel">
                    <div className="panel-header">
                        <div><h2>渠道占比</h2><p>按请求数排序</p></div>
                        <span className="flex items-center gap-1 text-xs text-muted"><ArrowDownRight size={14} />失败 0.8%</span>
                    </div>
                    <div className="channel-usage-list">
                        {channelUsage.map((channel, index) => (
                            <div className="channel-usage-row" key={channel.id}>
                                <span className="font-mono text-xs text-subtle">{String(index + 1).padStart(2, "0")}</span>
                                <ProviderMark type={channel.type} size="sm" />
                                <div className="min-w-0 flex-1">
                                    <p className="truncate text-sm font-medium text-ink">{channel.name}</p>
                                    <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-soft">
                                        <span className="block h-full rounded-full bg-data-blue" style={{ width: `${[92, 68, 52, 24][index % 4]}%` }} />
                                    </div>
                                </div>
                                <strong className="font-mono text-xs text-ink">{formatNumber(channel.requests)}</strong>
                            </div>
                        ))}
                    </div>
                </article>
            </section>
        </div>
    );
}
