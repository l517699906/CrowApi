import { type FormEvent, type KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
    CalendarRange,
    CheckCircle2,
    ChevronDown,
    ChevronRight,
    Clock3,
    FilterX,
    Search,
    ShieldAlert,
    TriangleAlert,
} from "lucide-react";
import { type InfiniteData, useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { formatDateTime, formatDuration, formatNumber } from "../lib/format";
import { apiKeyApi, channelApi, logApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type { GetLogsInput, RequestLog } from "../types";
import { LogDetailDrawer } from "../components/LogDetailDrawer";
import { LogRiskBadge } from "../components/LogRiskBadge";
import { PageTitle, StatusBadge } from "../components/ui";

const PAGE_SIZE = 50;
const LIVE_BATCH_SIZE = 200;
const MAX_LIVE_BATCHES_PER_SYNC = 5;
const LIVE_SYNC_DELAY_MS = 200;
const LOG_CHANGED_EVENT = "request-logs-changed";

interface LogChangedEvent {
    latest_seq: number;
    pending: number;
    reset: boolean;
}

interface LogFilterState {
    apiKeyName: string;
    channelName: string;
    dateFrom: string;
    dateTo: string;
    keyword: string;
    model: string;
}

const EMPTY_FILTERS: LogFilterState = {
    apiKeyName: "",
    channelName: "",
    dateFrom: "",
    dateTo: "",
    keyword: "",
    model: "",
};

function uniqueSorted(values: Array<string | null | undefined>): string[] {
    return Array.from(new Set(values.filter((value): value is string => Boolean(value))))
        .sort((left, right) => left.localeCompare(right, "zh-CN"));
}

function toDateBoundary(value: string, endOfDay: boolean): string | undefined {
    if (!value) {
        return undefined;
    }

    const time = endOfDay ? "23:59:59.999" : "00:00:00.000";
    const date = new Date(`${value}T${time}`);
    return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function toGetLogsInput(filters: LogFilterState, offset?: number): GetLogsInput {
    return {
        limit: PAGE_SIZE,
        ...(offset === undefined ? {} : { offset }),
        ...(filters.keyword ? { keyword: filters.keyword } : {}),
        ...(filters.apiKeyName ? { api_key_name: filters.apiKeyName } : {}),
        ...(filters.channelName ? { channel_name: filters.channelName } : {}),
        ...(filters.model ? { model: filters.model } : {}),
        ...(filters.dateFrom ? { date_from: toDateBoundary(filters.dateFrom, false) } : {}),
        ...(filters.dateTo ? { date_to: toDateBoundary(filters.dateTo, true) } : {}),
    };
}

type LogsQueryData = InfiniteData<RequestLog[], number>;

function maxLogSeq(logs: RequestLog[]): number {
    return logs.reduce((max, log) => Math.max(max, log.seq ?? 0), 0);
}

function mergeLiveLogs(data: LogsQueryData, incoming: RequestLog[]): LogsQueryData {
    if (incoming.length === 0 || data.pages.length === 0) {
        return data;
    }

    const existingIds = new Set(data.pages.flatMap((page) => page.map((log) => log.id)));
    const fresh = incoming
        .filter((log) => !existingIds.has(log.id))
        .sort((left, right) => (right.seq ?? 0) - (left.seq ?? 0));

    if (fresh.length === 0) {
        return data;
    }

    return {
        ...data,
        pages: [[...fresh, ...data.pages[0]], ...data.pages.slice(1)],
    };
}

function LogStatus({ statusCode }: { statusCode: number }) {
    if (statusCode >= 500) {
        return <StatusBadge status="danger">{statusCode}</StatusBadge>;
    }
    if (statusCode >= 400) {
        return <StatusBadge status="warning">{statusCode}</StatusBadge>;
    }
    return <StatusBadge status="success">{statusCode}</StatusBadge>;
}

export function LogsPage() {
    const [draftFilters, setDraftFilters] = useState<LogFilterState>(EMPTY_FILTERS);
    const [appliedFilters, setAppliedFilters] = useState<LogFilterState>(EMPTY_FILTERS);
    const [filterError, setFilterError] = useState("");
    const [selectedLog, setSelectedLog] = useState<RequestLog | null>(null);
    const [pendingLiveLogs, setPendingLiveLogs] = useState(0);
    const queryClient = useQueryClient();
    const logsQueryKey = useMemo(
        () => [...queryKeys.logs, "paged", appliedFilters] as const,
        [appliedFilters],
    );
    const logsQueryKeyRef = useRef(logsQueryKey);
    const syncedSeqRef = useRef(0);
    const targetSeqRef = useRef(0);
    const liveSyncTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const liveSyncingRef = useRef(false);
    const liveSyncAgainRef = useRef(false);
    const queueLiveSyncRef = useRef<() => void>(() => undefined);
    logsQueryKeyRef.current = logsQueryKey;

    const logsQuery = useInfiniteQuery({
        queryKey: logsQueryKey,
        queryFn: ({ pageParam }) => logApi.getAll(toGetLogsInput(appliedFilters, pageParam)),
        initialPageParam: 0,
        getNextPageParam: (lastPage, pages) => {
            if (lastPage.length < PAGE_SIZE) {
                return undefined;
            }
            return pages.reduce((count, page) => count + page.length, 0);
        },
    });
    const { data: apiKeys = [] } = useQuery({
        queryKey: queryKeys.apiKeys,
        queryFn: apiKeyApi.getAll,
    });
    const { data: channels = [] } = useQuery({
        queryKey: queryKeys.channels,
        queryFn: channelApi.getAll,
    });
    const logs = useMemo(() => logsQuery.data?.pages.flat() ?? [], [logsQuery.data]);
    const apiKeyOptions = useMemo(() => uniqueSorted([
        ...apiKeys.map((apiKey) => apiKey.name),
        ...logs.map((log) => log.api_key_name),
    ]), [apiKeys, logs]);
    const channelOptions = useMemo(() => uniqueSorted([
        ...channels.map((channel) => channel.name),
        ...logs.map((log) => log.channel_name),
    ]), [channels, logs]);
    const modelOptions = useMemo(() => uniqueSorted([
        ...channels.flatMap((channel) => channel.models),
        ...logs.flatMap((log) => [log.model, log.upstream_model]),
    ]), [channels, logs]);
    const metrics = useMemo(() => {
        const totals = logs.reduce((result, log) => {
            result.latency += log.duration_ms;
            result.failures += log.status_code >= 400 ? 1 : 0;
            result.audited += log.risk_level === "clean" ? 0 : 1;
            return result;
        }, { latency: 0, failures: 0, audited: 0 });

        return {
            ...totals,
            averageLatency: logs.length > 0 ? Math.round(totals.latency / logs.length) : 0,
            successRate: logs.length > 0 ? (1 - totals.failures / logs.length) * 100 : null,
        };
    }, [logs]);
    const hasDraftFilters = Object.values(draftFilters).some(Boolean);
    const loadedPageCount = logs.length > 0 ? Math.ceil(logs.length / PAGE_SIZE) : 0;

    const syncLiveLogs = useCallback(async () => {
        if (liveSyncingRef.current) {
            liveSyncAgainRef.current = true;
            return;
        }

        const queryData = queryClient.getQueryData<LogsQueryData>(logsQueryKey);
        if (!queryData) {
            liveSyncAgainRef.current = true;
            return;
        }

        const target = targetSeqRef.current;
        if (target <= syncedSeqRef.current) {
            setPendingLiveLogs(0);
            return;
        }

        liveSyncingRef.current = true;
        let failed = false;
        try {
            let cursor = syncedSeqRef.current;
            let batchCount = 0;

            while (cursor < target && batchCount < MAX_LIVE_BATCHES_PER_SYNC) {
                const rows = await logApi.getAll({
                    ...toGetLogsInput(appliedFilters),
                    limit: LIVE_BATCH_SIZE,
                    after_seq: cursor,
                });

                if (rows.length === 0) {
                    cursor = target;
                    break;
                }

                const nextCursor = maxLogSeq(rows);
                if (nextCursor <= cursor) {
                    cursor = target;
                    break;
                }

                queryClient.setQueryData<LogsQueryData>(logsQueryKey, (current) => (
                    current ? mergeLiveLogs(current, rows) : current
                ));
                cursor = nextCursor;
                batchCount += 1;

                if (rows.length < LIVE_BATCH_SIZE) {
                    // SQL 按 seq 升序返回，短页说明已经追上当前游标。
                    cursor = Math.max(cursor, target);
                    break;
                }
            }

            syncedSeqRef.current = Math.max(syncedSeqRef.current, cursor);
            if (syncedSeqRef.current >= targetSeqRef.current) {
                setPendingLiveLogs(0);
            } else {
                setPendingLiveLogs(Math.max(1, targetSeqRef.current - syncedSeqRef.current));
            }
        } catch {
            // 保留目标游标，下一次事件或窗口聚焦时可以安全重试。
            failed = true;
        } finally {
            liveSyncingRef.current = false;
            if (!failed && (liveSyncAgainRef.current || targetSeqRef.current > syncedSeqRef.current)) {
                liveSyncAgainRef.current = false;
                queueLiveSyncRef.current();
            }
        }
    }, [appliedFilters, logsQueryKey, queryClient]);

    const queueLiveSync = useCallback(() => {
        if (liveSyncTimerRef.current !== null) {
            return;
        }

        liveSyncTimerRef.current = setTimeout(() => {
            liveSyncTimerRef.current = null;
            void syncLiveLogs();
        }, LIVE_SYNC_DELAY_MS);
    }, [syncLiveLogs]);
    queueLiveSyncRef.current = queueLiveSync;

    useEffect(() => {
        const loadedSeq = maxLogSeq(logs);
        if (loadedSeq > syncedSeqRef.current) {
            syncedSeqRef.current = loadedSeq;
        }
        if (targetSeqRef.current > syncedSeqRef.current) {
            queueLiveSync();
        }
    }, [logs, queueLiveSync]);

    useEffect(() => {
        let disposed = false;
        let unlisten: (() => void) | undefined;

        const registerListener = async () => {
            try {
                const cleanup = await listen<LogChangedEvent>(LOG_CHANGED_EVENT, ({ payload }) => {
                    if (payload.reset) {
                        syncedSeqRef.current = 0;
                        targetSeqRef.current = 0;
                        liveSyncAgainRef.current = false;
                        setPendingLiveLogs(0);
                        void queryClient.invalidateQueries({
                            queryKey: logsQueryKeyRef.current,
                            exact: true,
                        });
                        return;
                    }

                    const latestSeq = Number(payload.latest_seq);
                    if (!Number.isFinite(latestSeq) || latestSeq <= targetSeqRef.current) {
                        return;
                    }

                    targetSeqRef.current = latestSeq;
                    setPendingLiveLogs((current) => Math.max(current, payload.pending || 1));
                    queueLiveSyncRef.current();
                });

                if (disposed) {
                    cleanup();
                } else {
                    unlisten = cleanup;
                }
            } catch {
                // Web 预览没有 Tauri 事件桥，初始查询仍然可以正常工作。
            }
        };

        void registerListener();
        return () => {
            disposed = true;
            if (liveSyncTimerRef.current !== null) {
                clearTimeout(liveSyncTimerRef.current);
                liveSyncTimerRef.current = null;
            }
            unlisten?.();
        };
    }, [queryClient]);

    const updateDraftFilter = <Key extends keyof LogFilterState>(key: Key, value: LogFilterState[Key]) => {
        setDraftFilters((current) => ({ ...current, [key]: value }));
        if (key === "dateFrom" || key === "dateTo") {
            setFilterError("");
        }
    };

    const applyFilters = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        if (draftFilters.dateFrom && draftFilters.dateTo && draftFilters.dateFrom > draftFilters.dateTo) {
            setFilterError("开始日期不能晚于结束日期");
            return;
        }

        const nextFilters = { ...draftFilters, keyword: draftFilters.keyword.trim() };
        setFilterError("");
        setSelectedLog(null);
        if (JSON.stringify(nextFilters) === JSON.stringify(appliedFilters)) {
            void logsQuery.refetch();
            return;
        }
        setAppliedFilters(nextFilters);
    };

    const resetFilters = () => {
        setDraftFilters(EMPTY_FILTERS);
        setFilterError("");
        setSelectedLog(null);
        if (JSON.stringify(appliedFilters) === JSON.stringify(EMPTY_FILTERS)) {
            void logsQuery.refetch();
        } else {
            setAppliedFilters(EMPTY_FILTERS);
        }
    };

    const openLogFromKeyboard = (event: KeyboardEvent<HTMLTableRowElement>, log: RequestLog) => {
        if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            setSelectedLog(log);
        }
    };

    return (
        <div className="page-enter">
            <PageTitle
                title="请求日志"
                meta={`已加载 ${formatNumber(logs.length)} 条 · 每页 ${PAGE_SIZE} 条`}
            />

            <form className="toolbar-row log-filter-bar" onSubmit={applyFilters}>
                <label className="search-field log-search">
                    <span className="sr-only">关键词</span>
                    <Search size={16} />
                    <input
                        value={draftFilters.keyword}
                        placeholder="搜索请求 ID、密钥、渠道或模型"
                        onChange={(event) => updateDraftFilter("keyword", event.target.value)}
                    />
                </label>
                <select
                    className="filter-select"
                    aria-label="按密钥筛选"
                    value={draftFilters.apiKeyName}
                    onChange={(event) => updateDraftFilter("apiKeyName", event.target.value)}
                >
                    <option value="">全部密钥</option>
                    {apiKeyOptions.map((item) => <option key={item} value={item}>{item}</option>)}
                </select>
                <select
                    className="filter-select"
                    aria-label="按渠道筛选"
                    value={draftFilters.channelName}
                    onChange={(event) => updateDraftFilter("channelName", event.target.value)}
                >
                    <option value="">全部渠道</option>
                    {channelOptions.map((item) => <option key={item} value={item}>{item}</option>)}
                </select>
                <select
                    className="filter-select"
                    aria-label="按模型筛选"
                    value={draftFilters.model}
                    onChange={(event) => updateDraftFilter("model", event.target.value)}
                >
                    <option value="">全部模型</option>
                    {modelOptions.map((item) => <option key={item} value={item}>{item}</option>)}
                </select>
                <div className="date-range-filter" role="group" aria-label="日期范围">
                    <CalendarRange size={15} />
                    <label>
                        <span className="sr-only">开始日期</span>
                        <input
                            type="date"
                            value={draftFilters.dateFrom}
                            max={draftFilters.dateTo || undefined}
                            onChange={(event) => updateDraftFilter("dateFrom", event.target.value)}
                        />
                    </label>
                    <span aria-hidden="true">至</span>
                    <label>
                        <span className="sr-only">结束日期</span>
                        <input
                            type="date"
                            value={draftFilters.dateTo}
                            min={draftFilters.dateFrom || undefined}
                            onChange={(event) => updateDraftFilter("dateTo", event.target.value)}
                        />
                    </label>
                </div>
                <button type="submit" className="button-primary log-query-button">
                    {logsQuery.isFetching && !logsQuery.isFetchingNextPage ? <span className="button-spinner is-inverse" /> : <Search size={15} />}
                    查询
                </button>
                {hasDraftFilters ? (
                    <button type="button" className="button-ghost" onClick={resetFilters}>
                        <FilterX size={15} />清除
                    </button>
                ) : null}
            </form>
            {filterError ? <p className="filter-error" role="alert">{filterError}</p> : null}

            <section className="log-stats-strip mt-4" aria-label="已加载日志概览">
                <div><CheckCircle2 size={17} className="text-accent" /><span>成功率</span><strong>{metrics.successRate === null ? "--" : `${metrics.successRate.toFixed(1)}%`}</strong></div>
                <div><Clock3 size={17} className="text-data-blue" /><span>平均延迟</span><strong>{formatDuration(metrics.averageLatency)}</strong></div>
                <div><TriangleAlert size={17} className="text-warning" /><span>失败请求</span><strong>{metrics.failures}</strong></div>
                <div><ShieldAlert size={17} className="text-coral" /><span>安全审计</span><strong>{metrics.audited}</strong></div>
            </section>

            <section className="panel mt-4 min-w-0">
                <div className="panel-header panel-header-compact">
                    <p className="text-xs text-muted">已加载 {formatNumber(logs.length)} 条 · {loadedPageCount} 页</p>
                    <span className="flex items-center gap-1.5 text-xs text-muted">
                        <span className="live-dot" />
                        {pendingLiveLogs > 0 ? `同步中 ${formatNumber(pendingLiveLogs)} 条` : "最新请求优先"}
                    </span>
                </div>
                <div className="table-scroll">
                    <table className="data-table log-table min-w-[840px]">
                        <thead>
                            <tr>
                                <th>序号</th>
                                <th>时间</th>
                                <th>密钥名</th>
                                <th>渠道名</th>
                                <th>模型</th>
                                <th>Token</th>
                                <th>延迟</th>
                                <th>状态码</th>
                                <th>安全等级</th>
                            </tr>
                        </thead>
                        <tbody>
                            {logs.map((log) => (
                                <tr
                                    key={log.id}
                                    className="log-row"
                                    tabIndex={0}
                                    aria-haspopup="dialog"
                                    aria-label={`查看请求 ${log.seq ?? log.id} 详情`}
                                    onClick={() => setSelectedLog(log)}
                                    onKeyDown={(event) => openLogFromKeyboard(event, log)}
                                >
                                    <td className="font-mono text-xs font-semibold text-ink">{log.seq === null ? "--" : `#${log.seq}`}</td>
                                    <td className="font-mono text-[11px] text-muted">{formatDateTime(log.created_at)}</td>
                                    <td className="text-xs text-ink">{log.api_key_name ?? "未识别"}</td>
                                    <td className="text-xs text-ink">{log.channel_name ?? "未路由"}</td>
                                    <td>
                                        <span className="model-name">{log.model}</span>
                                        {log.is_stream ? <span className="ml-1.5 text-[10px] text-subtle">STREAM</span> : null}
                                    </td>
                                    <td>
                                        <span className="log-token-pair">
                                            <span>{formatNumber(log.prompt_tokens)}</span>
                                            <b>+</b>
                                            <span>{formatNumber(log.completion_tokens)}</span>
                                        </span>
                                    </td>
                                    <td className="font-mono text-xs text-ink">{formatDuration(log.duration_ms)}</td>
                                    <td>
                                        <LogStatus statusCode={log.status_code} />
                                        {log.is_retry ? <span className="ml-1.5 text-[10px] text-warning">重试</span> : null}
                                    </td>
                                    <td>
                                        <span className="log-risk-cell">
                                            <LogRiskBadge level={log.risk_level} />
                                            <ChevronRight size={15} aria-hidden="true" />
                                        </span>
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
                {logsQuery.isPending ? (
                    <div className="empty-state"><span className="button-spinner" /><strong>正在读取日志</strong></div>
                ) : logsQuery.error && logs.length === 0 ? (
                    <div className="empty-state"><TriangleAlert size={22} /><strong>日志读取失败</strong><span>{errorMessage(logsQuery.error)}</span></div>
                ) : logs.length === 0 ? (
                    <div className="empty-state">
                        <Search size={22} />
                        <strong>没有匹配的请求</strong>
                        <span>调整筛选条件后重新查询</span>
                    </div>
                ) : (
                    <footer className="log-pagination">
                        <span>每次加载 {PAGE_SIZE} 条，当前共 {formatNumber(logs.length)} 条</span>
                        {logsQuery.isFetchNextPageError ? <span className="text-danger">{errorMessage(logsQuery.error)}</span> : null}
                        <button
                            type="button"
                            className="button-secondary"
                            disabled={!logsQuery.hasNextPage || logsQuery.isFetchingNextPage}
                            onClick={() => void logsQuery.fetchNextPage()}
                        >
                            {logsQuery.isFetchingNextPage ? <span className="button-spinner" /> : <ChevronDown size={15} />}
                            {logsQuery.isFetchingNextPage ? "加载中" : logsQuery.hasNextPage ? "加载更多" : "已加载全部"}
                        </button>
                    </footer>
                )}
            </section>

            {selectedLog ? <LogDetailDrawer log={selectedLog} onClose={() => setSelectedLog(null)} /> : null}
        </div>
    );
}
