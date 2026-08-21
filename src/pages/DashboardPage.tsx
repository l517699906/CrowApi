import { useMemo } from "react";
import {
    Activity,
    AlertTriangle,
    ArrowUpRight,
    CircleGauge,
    Clock3,
    Coins,
    Cpu,
    Database,
    KeyRound,
    LoaderCircle,
    MessageSquare,
    Radio,
    RefreshCw,
    Route,
    Server,
    Zap,
} from "lucide-react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { formatCompactNumber, formatDateTime, formatDuration, formatNumber } from "../lib/format";
import { apiKeyApi, channelApi, logApi, serverApi, statsApi } from "../lib/api";
import { nextPollingInterval, normalizeAppError, queryKeys } from "../lib/query";
import { getProtocolMeta, protocolGradient, protocolTotal } from "../lib/protocol";
import {
    localDayStatsInput,
    materializeUsageSeries,
    rollingUsageCacheRange,
    rollingUsageStatsInput,
} from "../lib/statistics";
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
        key: "totalRequests",
        label: "累计请求",
        note: "全部历史请求",
        trend: "neutral",
        icon: Database,
        tone: "green",
    },
    {
        key: "totalTokens",
        label: "累计 Token",
        note: "全部历史用量",
        trend: "neutral",
        icon: Coins,
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
    loading?: boolean;
}

function KpiCard({ label, value, note, trend, tone, icon: Icon, loading = false }: KpiCardProps) {
    return (
        <article className="kpi-card" aria-busy={loading}>
            <div className={`kpi-icon kpi-${tone}`}>
                <Icon size={19} strokeWidth={1.8} />
            </div>
            <div className="mt-5 flex items-end justify-between gap-3">
                <div>
                    <p className="text-xs font-medium text-muted">{label}</p>
                    <p className={`mt-1 font-mono text-[26px] font-semibold leading-9 text-ink ${loading ? "dashboard-value-loading" : ""}`}>
                        {value}
                    </p>
                </div>
                {trend === "up" ? <ArrowUpRight className="mb-1 text-accent" size={18} /> : null}
            </div>
            <p className="mt-2 text-xs text-subtle">{note}</p>
        </article>
    );
}

interface DashboardFailure {
    source: string;
    error: Error;
    retry: () => void;
}

function DashboardAlerts({ failures }: { failures: DashboardFailure[] }) {
    if (failures.length === 0) return null;

    return (
        <section className="dashboard-alerts" aria-label="仪表盘数据源错误">
            {failures.map(({ source, error, retry }) => {
                const normalized = normalizeAppError(error);
                return (
                    <div className="dashboard-alert" key={source} role="alert">
                        <AlertTriangle size={15} aria-hidden="true" />
                        <div>
                            <strong>{source}</strong>
                            <span>{normalized.message}</span>
                            <small>
                                {normalized.code}
                                {normalized.trace_id ? ` · Trace ${normalized.trace_id}` : ""}
                            </small>
                        </div>
                        <button type="button" onClick={retry} title={`重新读取${source}`} aria-label={`重新读取${source}`}>
                            <RefreshCw size={14} />
                        </button>
                    </div>
                );
            })}
        </section>
    );
}

function PanelDataState({ label, failed = false }: { label: string; failed?: boolean }) {
    return (
        <div className={`dashboard-data-state ${failed ? "dashboard-data-state-error" : ""}`}>
            {failed ? <AlertTriangle size={18} /> : <LoaderCircle className="dashboard-spinner" size={18} />}
            <span>{label}</span>
        </div>
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
    const now = new Date();
    const dayRange = localDayStatsInput(now);
    const rollingRange = rollingUsageStatsInput(24, 3_600, now);
    const rollingCacheRange = rollingUsageCacheRange(24, 3_600, now);
    const statsQuery = useQuery({
        queryKey: queryKeys.dashboardStats(dayRange.date_from!, dayRange.date_to!),
        queryFn: () => statsApi.getDashboard(dayRange),
        refetchInterval: ({ state }) => nextPollingInterval(5_000, state.fetchFailureCount),
    });
    const rollingStatsQuery = useQuery({
        queryKey: queryKeys.usageStats(
            "dashboard-24h",
            rollingCacheRange.dateFrom,
            rollingCacheRange.dateTo,
        ),
        queryFn: () => statsApi.getUsage(rollingRange),
        refetchInterval: ({ state }) => nextPollingInterval(5_000, state.fetchFailureCount),
    });
    const channelsQuery = useQuery({
        queryKey: queryKeys.channels,
        queryFn: channelApi.getAll,
    });
    const logsQuery = useQuery({
        queryKey: queryKeys.logs,
        queryFn: () => logApi.getAll({ limit: 6 }),
        refetchInterval: ({ state }) => nextPollingInterval(3_000, state.fetchFailureCount),
    });
    const apiKeysQuery = useQuery({
        queryKey: queryKeys.apiKeys,
        queryFn: apiKeyApi.getAll,
    });
    const serverStatusQuery = useQuery({
        queryKey: queryKeys.serverStatus,
        queryFn: serverApi.getStatus,
        refetchInterval: ({ state }) => nextPollingInterval(2_000, state.fetchFailureCount),
    });
    const stats = statsQuery.data;
    const rollingStats = rollingStatsQuery.data;
    const channels = channelsQuery.data ?? [];
    const logs = logsQuery.data ?? [];
    const apiKeys = apiKeysQuery.data ?? [];
    const serverStatus = serverStatusQuery.data;
    const activeChannels = useMemo(
        () => channels.filter((channel) => channel.status === 1).length,
        [channels],
    );
    const recentLogs = logs.slice(0, 6);
    const requestSeries = useMemo(
        () => materializeUsageSeries(rollingStats?.series ?? [], 24),
        [rollingStats?.series],
    );
    const chartMax = Math.max(...requestSeries, 1);
    const rollingRequests = rollingStats?.total_requests ?? 0;
    const rollingFailures = rollingStats?.failed_requests ?? 0;
    const successRate = rollingRequests > 0
        ? ((rollingRequests - rollingFailures) / rollingRequests) * 100
        : null;
    const testedChannels = channels.filter((channel) => channel.last_test_ok !== null);
    const healthyChannels = testedChannels.filter((channel) => channel.last_test_ok === 1).length;
    const healthRate = testedChannels.length > 0 ? (healthyChannels / testedChannels.length) * 100 : 0;
    const activeKeys = apiKeysQuery.data
        ? apiKeys.filter((key) => key.status === 1 && (!key.expires_at || new Date(key.expires_at).getTime() > Date.now())).length
        : null;
    const protocolStats = stats?.protocols ?? [];
    const totalProtocolRequests = protocolTotal(protocolStats);
    const queryFailures: DashboardFailure[] = [
        statsQuery.error ? {
            source: "概览统计",
            error: statsQuery.error,
            retry: () => { void statsQuery.refetch(); },
        } : null,
        rollingStatsQuery.error ? {
            source: "24 小时趋势",
            error: rollingStatsQuery.error,
            retry: () => { void rollingStatsQuery.refetch(); },
        } : null,
        channelsQuery.error ? {
            source: "渠道状态",
            error: channelsQuery.error,
            retry: () => { void channelsQuery.refetch(); },
        } : null,
        logsQuery.error ? {
            source: "最近请求",
            error: logsQuery.error,
            retry: () => { void logsQuery.refetch(); },
        } : null,
        apiKeysQuery.error ? {
            source: "API 密钥",
            error: apiKeysQuery.error,
            retry: () => { void apiKeysQuery.refetch(); },
        } : null,
        serverStatusQuery.error ? {
            source: "网关状态",
            error: serverStatusQuery.error,
            retry: () => { void serverStatusQuery.refetch(); },
        } : null,
    ].filter((failure): failure is DashboardFailure => failure !== null);

    const statValue = (key: string) => {
        switch (key) {
            case "requests": return stats ? formatNumber(stats.today_requests) : "--";
            case "tokens": return stats ? formatCompactNumber(stats.today_total_tokens) : "--";
            case "totalRequests": return stats ? formatCompactNumber(stats.total_requests) : "--";
            case "totalTokens": return stats ? formatCompactNumber(stats.total_tokens) : "--";
            case "channels": return channelsQuery.data ? `${activeChannels} / ${channels.length}` : "--";
            case "latency": return stats ? formatDuration(Math.round(stats.avg_latency_ms)) : "--";
            default: return "--";
        }
    };

    const serverBadge = serverStatus
        ? {
            status: serverStatus.running ? "success" as const : "danger" as const,
            label: serverStatus.running ? "服务正常" : "服务未连接",
        }
        : serverStatusQuery.isPending
            ? { status: "neutral" as const, label: "状态读取中" }
            : { status: "danger" as const, label: "状态不可用" };

    return (
        <div className="page-enter">
            <PageTitle
                title="仪表盘"
                meta={`今天 · ${new Intl.DateTimeFormat("zh-CN", { month: "long", day: "numeric", weekday: "short" }).format(new Date())}`}
                action={(
                    <StatusBadge status={serverBadge.status} dot>
                        {serverBadge.label}
                    </StatusBadge>
                )}
            />

            <DashboardAlerts failures={queryFailures} />

            <section className="gateway-flow" aria-label="网关请求链路">
                <div className="flow-node">
                    <span className="flow-icon"><Server size={17} /></span>
                    <div>
                        <strong>本地入口</strong>
                        <span>{serverStatus?.url?.replace(/^https?:\/\//, "") ?? "--"}</span>
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
                        <strong>{channelsQuery.data ? `${activeChannels} 条上游` : "-- 条上游"}</strong>
                        <span>
                            {channelsQuery.data
                                ? testedChannels.length > 0
                                    ? `测试通过率 ${healthRate.toFixed(0)}%`
                                    : "尚未测试"
                                : channelsQuery.isPending ? "渠道读取中" : "渠道不可用"}
                        </span>
                    </div>
                </div>
                <div className="ml-auto hidden items-center gap-5 xl:flex">
                    <div className="flow-metric">
                        <span>24h 成功率</span>
                        <strong>{successRate === null ? "--" : `${successRate.toFixed(1)}%`}</strong>
                    </div>
                    <div className="flow-metric">
                        <span>24h 请求</span>
                        <strong>{rollingStats ? formatCompactNumber(rollingRequests) : "--"}</strong>
                    </div>
                </div>
            </section>

            <section className="kpi-grid" aria-label="请求与网关概览">
                {statDefinitions.map(({ key, ...stat }) => {
                    const sourceQuery = key === "channels" ? channelsQuery : statsQuery;
                    const hasData = key === "channels" ? Boolean(channelsQuery.data) : Boolean(stats);
                    return (
                        <KpiCard
                            key={key}
                            {...stat}
                            value={statValue(key)}
                            loading={sourceQuery.isPending}
                            note={hasData
                                ? stat.note
                                : sourceQuery.isPending ? "正在读取" : "数据暂不可用"}
                        />
                    );
                })}
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
                            <span className="font-mono font-semibold text-ink">
                                {rollingStats ? formatNumber(rollingRequests) : "--"}
                            </span>
                        </div>
                    </div>
                    {rollingStatsQuery.isPending ? (
                        <PanelDataState label="正在读取 24 小时趋势" />
                    ) : rollingStats ? (
                        <>
                            <div className="traffic-chart" aria-label="24 小时请求趋势柱状图">
                                {requestSeries.map((value, index) => (
                                    <div className="chart-column" key={`${value}-${index}`}>
                                        <span
                                            className="chart-bar"
                                            style={{ height: `${Math.max((value / chartMax) * 100, 7)}%` }}
                                            title={`${23 - index} 小时前 · ${value} 次`}
                                            role="img"
                                            aria-label={`${23 - index} 小时前，${value} 次请求`}
                                        />
                                    </div>
                                ))}
                            </div>
                            <div className="chart-axis">
                                <span>24 小时前</span><span>18 小时前</span><span>12 小时前</span><span>6 小时前</span><span>现在</span>
                            </div>
                        </>
                    ) : (
                        <PanelDataState label="趋势数据暂不可用" failed />
                    )}
                </article>

                <article className="panel">
                    <div className="panel-header">
                        <div>
                            <h2>渠道状态</h2>
                            <p>{channelsQuery.data ? `${activeChannels} 条线路参与路由` : "线路状态尚未读取"}</p>
                        </div>
                        <Link className="panel-link" to="/channels">管理</Link>
                    </div>
                    <div className="channel-health-list">
                        {channelsQuery.isPending ? (
                            <PanelDataState label="正在读取渠道状态" />
                        ) : !channelsQuery.data ? (
                            <PanelDataState label="渠道状态暂不可用" failed />
                        ) : channels.length === 0 ? (
                            <div className="dashboard-compact-empty">尚未配置渠道</div>
                        ) : channels.map((channel) => (
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

            <section className="panel protocol-panel mt-4">
                <div className="panel-header">
                    <div>
                        <h2>协议分布</h2>
                        <p>按累计请求类型统计</p>
                    </div>
                    <span className="font-mono text-xs text-muted">
                        {stats ? `${formatNumber(totalProtocolRequests)} 次请求` : "--"}
                    </span>
                </div>
                {statsQuery.isPending ? (
                    <PanelDataState label="正在读取协议分布" />
                ) : stats ? (
                    <div className="protocol-distribution">
                    <div
                        className="protocol-ring"
                        style={{ background: protocolGradient(protocolStats) }}
                        role="img"
                        aria-label={`协议分布，共 ${totalProtocolRequests} 次请求`}
                    >
                        <div><strong>{formatCompactNumber(totalProtocolRequests)}</strong><span>累计请求</span></div>
                    </div>
                    <div className="protocol-legend">
                        {protocolStats.map((item) => {
                            const protocol = getProtocolMeta(item.mode);
                            const percentage = totalProtocolRequests > 0 ? (item.request_count / totalProtocolRequests) * 100 : 0;
                            return (
                                <div className="protocol-legend-row" key={item.mode}>
                                    <span className={`protocol-swatch protocol-swatch-${protocol.tone}`} />
                                    <div><strong>{protocol.label}</strong><span>{formatCompactNumber(item.total_tokens)} Token</span></div>
                                    <div><strong>{formatNumber(item.request_count)}</strong><span>{percentage.toFixed(1)}%</span></div>
                                </div>
                            );
                        })}
                        {protocolStats.length === 0 ? <p className="protocol-empty">暂无协议用量</p> : null}
                    </div>
                    </div>
                ) : (
                    <PanelDataState label="协议分布暂不可用" failed />
                )}
            </section>

            <section className="panel mt-4 min-w-0">
                <div className="panel-header">
                        <div>
                            <h2>最近请求</h2>
                            <p>{activeKeys === null ? "密钥状态尚未读取" : `${activeKeys} 个有效密钥`}</p>
                        </div>
                    <Link className="panel-link" to="/logs">查看全部</Link>
                </div>
                {logsQuery.isPending ? (
                    <PanelDataState label="正在读取最近请求" />
                ) : logsQuery.data ? (
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
                ) : (
                    <PanelDataState label="最近请求暂不可用" failed />
                )}
                {logsQuery.data && recentLogs.length === 0 ? (
                    <div className="empty-state"><Activity size={22} /><strong>暂无请求记录</strong><span>完成一次网关调用后会在这里显示</span></div>
                ) : null}
            </section>

            <div className="mt-4 grid gap-4 sm:grid-cols-3">
                <div className="mini-stat"><Activity size={17} /><span>今日请求</span><strong>{stats ? formatCompactNumber(stats.today_requests) : "--"}</strong></div>
                <div className="mini-stat"><KeyRound size={17} /><span>有效密钥</span><strong>{activeKeys ?? "--"}</strong></div>
                <div className="mini-stat"><Clock3 size={17} /><span>网关状态</span><strong>{serverStatus ? serverStatus.running ? "运行中" : "未连接" : "--"}</strong></div>
            </div>
        </div>
    );
}
