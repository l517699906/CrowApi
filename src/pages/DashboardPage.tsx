import { useMemo } from "react";
import {
    Activity,
    ArrowUpRight,
    CircleGauge,
    Clock3,
    Cpu,
    KeyRound,
    MessageSquare,
    Radio,
    Route,
    Server,
    Zap,
} from "lucide-react";
import { Link } from "react-router-dom";
import { requestSeries } from "../data/mockData";
import { formatCompactNumber, formatDateTime, formatDuration, formatNumber } from "../lib/format";
import { useGatewayStore } from "../store/gatewayStore";
import type { RequestLog } from "../types";
import { PageTitle, ProviderMark, StatusBadge } from "../components/ui";

const statDefinitions = [
    {
        label: "今日请求数",
        value: "2,841",
        note: "较昨日 12.8%",
        trend: "up",
        icon: MessageSquare,
        tone: "green",
    },
    {
        label: "Token 消耗",
        value: "2.95M",
        note: "输入 68% · 输出 32%",
        trend: "neutral",
        icon: Cpu,
        tone: "blue",
    },
    {
        label: "活跃渠道数",
        value: "dynamic",
        note: "可参与路由",
        trend: "neutral",
        icon: Radio,
        tone: "amber",
    },
    {
        label: "平均延迟",
        value: "1.24 s",
        note: "较昨日降低 86 ms",
        trend: "up",
        icon: CircleGauge,
        tone: "coral",
    },
] as const;

interface KpiCardProps {
    label: string;
    value: string;
    note: string;
    trend: "up" | "neutral";
    tone: string;
    icon: typeof Activity;
}

function KpiCard({ label, value, note, trend, tone, icon: Icon }: KpiCardProps) {
    return (
        <article className="kpi-card">
            <div className={`kpi-icon kpi-${tone}`}>
                <Icon size={19} strokeWidth={1.8} />
            </div>
            <div className="mt-5 flex items-end justify-between gap-3">
                <div>
                    <p className="text-xs font-medium text-muted">{label}</p>
                    <p className="mt-1 font-mono text-[26px] font-semibold leading-9 text-ink">{value}</p>
                </div>
                {trend === "up" ? <ArrowUpRight className="mb-1 text-accent" size={18} /> : null}
            </div>
            <p className="mt-2 text-xs text-subtle">{note}</p>
        </article>
    );
}

function RequestStatus({ log }: { log: RequestLog }) {
    if (log.status_code >= 500) {
        return <StatusBadge status="danger">{log.status_code}</StatusBadge>;
    }
    if (log.status_code >= 400) {
        return <StatusBadge status="warning">{log.status_code}</StatusBadge>;
    }
    return <StatusBadge status="success">{log.status_code}</StatusBadge>;
}

export function DashboardPage() {
    const channels = useGatewayStore((state) => state.channels);
    const logs = useGatewayStore((state) => state.logs);
    const apiKeys = useGatewayStore((state) => state.apiKeys);
    const activeChannels = useMemo(
        () => channels.filter((channel) => channel.status === 1).length,
        [channels],
    );
    const recentLogs = logs.slice(0, 6);
    const chartMax = Math.max(...requestSeries["24h"]);

    return (
        <div className="page-enter">
            <PageTitle
                title="仪表盘"
                meta={`今天 · ${new Intl.DateTimeFormat("zh-CN", { month: "long", day: "numeric", weekday: "short" }).format(new Date())}`}
                action={(
                    <StatusBadge status="success" dot>
                        服务正常
                    </StatusBadge>
                )}
            />

            <section className="gateway-flow" aria-label="网关请求链路">
                <div className="flow-node">
                    <span className="flow-icon"><Server size={17} /></span>
                    <div>
                        <strong>本地入口</strong>
                        <span>127.0.0.1:8317</span>
                    </div>
                </div>
                <div className="flow-connector"><span /><span /><span /></div>
                <div className="flow-node">
                    <span className="flow-icon flow-icon-blue"><Route size={17} /></span>
                    <div>
                        <strong>智能路由</strong>
                        <span>加权优先级</span>
                    </div>
                </div>
                <div className="flow-connector"><span /><span /><span /></div>
                <div className="flow-node">
                    <span className="flow-icon flow-icon-amber"><Zap size={17} /></span>
                    <div>
                        <strong>{activeChannels} 条上游</strong>
                        <span>平均健康度 96%</span>
                    </div>
                </div>
                <div className="ml-auto hidden items-center gap-5 xl:flex">
                    <div className="flow-metric"><span>成功率</span><strong>99.2%</strong></div>
                    <div className="flow-metric"><span>队列</span><strong>0</strong></div>
                </div>
            </section>

            <section className="kpi-grid" aria-label="今日概览">
                {statDefinitions.map((stat) => (
                    <KpiCard
                        key={stat.label}
                        {...stat}
                        value={stat.value === "dynamic" ? `${activeChannels} / ${channels.length}` : stat.value}
                    />
                ))}
            </section>

            <section className="dashboard-primary-grid">
                <article className="panel min-w-0">
                    <div className="panel-header">
                        <div>
                            <h2>请求趋势</h2>
                            <p>最近 24 小时</p>
                        </div>
                        <div className="flex items-center gap-4 text-xs text-muted">
                            <span className="flex items-center gap-1.5"><span className="legend-dot legend-green" />请求</span>
                            <span className="font-mono font-semibold text-ink">2,841</span>
                        </div>
                    </div>
                    <div className="traffic-chart" aria-label="24 小时请求趋势柱状图">
                        {requestSeries["24h"].map((value, index) => (
                            <div className="chart-column" key={`${value}-${index}`}>
                                <span
                                    className="chart-bar"
                                    style={{ height: `${Math.max((value / chartMax) * 100, 7)}%` }}
                                    title={`${String(index).padStart(2, "0")}:00 · ${value} 次`}
                                />
                            </div>
                        ))}
                    </div>
                    <div className="chart-axis">
                        <span>00:00</span><span>06:00</span><span>12:00</span><span>18:00</span><span>现在</span>
                    </div>
                </article>

                <article className="panel">
                    <div className="panel-header">
                        <div>
                            <h2>渠道状态</h2>
                            <p>{activeChannels} 条线路参与路由</p>
                        </div>
                        <Link className="panel-link" to="/channels">管理</Link>
                    </div>
                    <div className="channel-health-list">
                        {channels.map((channel) => (
                            <div className="channel-health-row" key={channel.id}>
                                <ProviderMark type={channel.type} size="sm" />
                                <div className="min-w-0 flex-1">
                                    <p className="truncate text-sm font-medium text-ink">{channel.name}</p>
                                    <p className="truncate text-xs text-subtle">{channel.models[0]}</p>
                                </div>
                                <div className="text-right">
                                    <p className="font-mono text-xs font-medium text-ink">
                                        {channel.status === 1 ? `${420 + channel.priority * 11} ms` : "--"}
                                    </p>
                                    <p className={`text-[11px] ${channel.status === 1 ? "text-accent" : "text-subtle"}`}>
                                        {channel.status === 1 ? "正常" : "已停用"}
                                    </p>
                                </div>
                            </div>
                        ))}
                    </div>
                </article>
            </section>

            <section className="panel mt-4 min-w-0">
                <div className="panel-header">
                    <div>
                        <h2>最近请求</h2>
                        <p>{apiKeys.filter((key) => key.status === 1).length} 个密钥正在使用</p>
                    </div>
                    <Link className="panel-link" to="/logs">查看全部</Link>
                </div>
                <div className="table-scroll">
                    <table className="data-table min-w-[760px]">
                        <thead>
                            <tr>
                                <th>时间</th>
                                <th>模型</th>
                                <th>渠道</th>
                                <th>状态</th>
                                <th>Token</th>
                                <th>延迟</th>
                            </tr>
                        </thead>
                        <tbody>
                            {recentLogs.map((log) => (
                                <tr key={log.id}>
                                    <td className="font-mono text-xs text-muted">{formatDateTime(log.created_at)}</td>
                                    <td><span className="model-name">{log.model}</span></td>
                                    <td>{log.channel_name}</td>
                                    <td><RequestStatus log={log} /></td>
                                    <td className="font-mono text-xs">{formatNumber(log.total_tokens)}</td>
                                    <td className="font-mono text-xs">{formatDuration(log.duration_ms)}</td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            </section>

            <div className="mt-4 grid gap-4 sm:grid-cols-3">
                <div className="mini-stat"><Activity size={17} /><span>总请求</span><strong>{formatCompactNumber(128_460)}</strong></div>
                <div className="mini-stat"><KeyRound size={17} /><span>有效密钥</span><strong>{apiKeys.filter((key) => key.status === 1).length}</strong></div>
                <div className="mini-stat"><Clock3 size={17} /><span>运行时间</span><strong>18d 7h</strong></div>
            </div>
        </div>
    );
}
