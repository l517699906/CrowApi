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
import { useQuery } from "@tanstack/react-query";
import { formatCompactNumber, formatDateTime, formatDuration, formatNumber } from "../lib/format";
import { apiKeyApi, channelApi, logApi, serverApi, statsApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type { RequestLog } from "../types";
import { PageTitle, ProviderMark, StatusBadge } from "../components/ui";

const statDefinitions = [
    {
        key: "requests",
        label: "今日请求数",
        note: "来自本地请求日志",
        trend: "neutral",
        icon: MessageSquare,
        tone: "green",
    },
    {
        key: "tokens",
        label: "Token 消耗",
        note: "今日累计用量",
        trend: "neutral",
        icon: Cpu,
        tone: "blue",
    },
    {
        key: "channels",
        label: "活跃渠道数",
        note: "可参与路由",
        trend: "neutral",
        icon: Radio,
        tone: "amber",
    },
    {
        key: "latency",
        label: "平均延迟",
        note: "今日请求平均值",
        trend: "neutral",
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
    const { data: stats, error: statsError } = useQuery({
        queryKey: queryKeys.dashboard,
        queryFn: statsApi.getDashboard,
        refetchInterval: 5_000,
    });
    const { data: channels = [], error: channelsError } = useQuery({
        queryKey: queryKeys.channels,
        queryFn: channelApi.getAll,
    });
    const { data: logs = [], error: logsError } = useQuery({
        queryKey: queryKeys.logs,
        queryFn: () => logApi.getAll({ limit: 500 }),
        refetchInterval: 3_000,
    });
    const { data: apiKeys = [], error: keysError } = useQuery({
        queryKey: queryKeys.apiKeys,
        queryFn: apiKeyApi.getAll,
    });
    const { data: serverStatus, error: serverError } = useQuery({
        queryKey: queryKeys.serverStatus,
        queryFn: serverApi.getStatus,
        refetchInterval: 2_000,
    });
    const activeChannels = useMemo(
        () => channels.filter((channel) => channel.status === 1).length,
        [channels],
    );
    const recentLogs = logs.slice(0, 6);
    const requestSeries = useMemo(() => {
        const values = Array.from({ length: 24 }, () => 0);
        const now = Date.now();
        logs.forEach((log) => {
            const elapsedHours = Math.floor((now - new Date(log.created_at).getTime()) / 3_600_000);
            if (elapsedHours >= 0 && elapsedHours < 24) {
                values[23 - elapsedHours] += 1;
            }
        });
        return values;
    }, [logs]);
    const chartMax = Math.max(...requestSeries, 1);
    const failures = logs.filter((log) => log.status_code >= 400).length;
    const successRate = logs.length > 0 ? ((logs.length - failures) / logs.length) * 100 : 100;
    const testedChannels = channels.filter((channel) => channel.last_test_ok !== null);
    const healthyChannels = testedChannels.filter((channel) => channel.last_test_ok === 1).length;
    const healthRate = testedChannels.length > 0 ? (healthyChannels / testedChannels.length) * 100 : 0;
    const activeKeys = apiKeys.filter((key) => key.status === 1 && (!key.expires_at || new Date(key.expires_at).getTime() > Date.now())).length;
    const queryError = statsError ?? channelsError ?? logsError ?? keysError ?? serverError;

    const statValue = (key: string) => {
        switch (key) {
            case "requests": return formatNumber(stats?.today_requests ?? 0);
            case "tokens": return formatCompactNumber(stats?.today_total_tokens ?? 0);
            case "channels": return `${activeChannels} / ${channels.length}`;
            case "latency": return formatDuration(Math.round(stats?.avg_latency_ms ?? 0));
            default: return "0";
        }
    };

    return (
        <div className="page-enter">
            <PageTitle
                title="仪表盘"
                meta={`今天 · ${new Intl.DateTimeFormat("zh-CN", { month: "long", day: "numeric", weekday: "short" }).format(new Date())}`}
                action={(
                    <StatusBadge status={serverStatus?.running ? "success" : "danger"} dot>
                        {serverStatus?.running ? "服务正常" : "服务未连接"}
                    </StatusBadge>
                )}
            />

            <section className="gateway-flow" aria-label="网关请求链路">
                <div className="flow-node">
                    <span className="flow-icon"><Server size={17} /></span>
                    <div>
                        <strong>本地入口</strong>
                        <span>{serverStatus?.url?.replace(/^https?:\/\//, "") ?? "127.0.0.1:8777"}</span>
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
                        <span>{testedChannels.length > 0 ? `测试通过率 ${healthRate.toFixed(0)}%` : "尚未测试"}</span>
                    </div>
                </div>
                <div className="ml-auto hidden items-center gap-5 xl:flex">
                    <div className="flow-metric"><span>成功率</span><strong>{successRate.toFixed(1)}%</strong></div>
                    <div className="flow-metric"><span>队列</span><strong>0</strong></div>
                </div>
            </section>

            <section className="kpi-grid" aria-label="今日概览">
                {statDefinitions.map(({ key, ...stat }) => (
                    <KpiCard
                        key={key}
                        {...stat}
                        value={statValue(key)}
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
                            <span className="font-mono font-semibold text-ink">{formatNumber(stats?.today_requests ?? 0)}</span>
                        </div>
                    </div>
                    <div className="traffic-chart" aria-label="24 小时请求趋势柱状图">
                        {requestSeries.map((value, index) => (
                            <div className="chart-column" key={`${value}-${index}`}>
                                <span
                                    className="chart-bar"
                                    style={{ height: `${Math.max((value / chartMax) * 100, 7)}%` }}
                                    title={`${23 - index} 小时前 · ${value} 次`}
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
                                        {channel.last_test_at ? formatDateTime(channel.last_test_at) : "--"}
                                    </p>
                                    <p className={`text-[11px] ${channel.last_test_ok === 1 ? "text-accent" : "text-subtle"}`}>
                                        {channel.status !== 1 ? "已停用" : channel.last_test_ok === 1 ? "测试通过" : channel.last_test_ok === 0 ? "测试失败" : "未测试"}
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
                        <p>{activeKeys} 个有效密钥</p>
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
                {recentLogs.length === 0 ? (
                    <div className="empty-state"><Activity size={22} /><strong>暂无请求记录</strong><span>完成一次网关调用后会在这里显示</span></div>
                ) : null}
            </section>

            <div className="mt-4 grid gap-4 sm:grid-cols-3">
                <div className="mini-stat"><Activity size={17} /><span>总请求</span><strong>{formatCompactNumber(stats?.total_requests ?? 0)}</strong></div>
                <div className="mini-stat"><KeyRound size={17} /><span>有效密钥</span><strong>{activeKeys}</strong></div>
                <div className="mini-stat"><Clock3 size={17} /><span>网关状态</span><strong>{serverStatus?.running ? "运行中" : "未连接"}</strong></div>
            </div>
            {queryError ? <p className="form-error mt-4" role="alert">{errorMessage(queryError)}</p> : null}
        </div>
    );
}
