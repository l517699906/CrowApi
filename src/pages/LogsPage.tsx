import { type FormEvent, type KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
    CalendarRange,
    CheckCircle2,
    ChevronDown,
    ChevronRight,
    Clock3,
    Download,
    Eraser,
    FilterX,
    FileJson2,
    Search,
    ShieldAlert,
    TableProperties,
    Trash2,
    TriangleAlert,
} from "lucide-react";
import { type InfiniteData, useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { formatDateTime, formatDuration, formatNumber, formatTokenCount } from "../lib/format";
import { apiKeyApi, channelApi, fileApi, logApi } from "../lib/api";
import { logExportName, logsToCsv, logsToJson } from "../lib/logExport";
import { getProtocolMeta } from "../lib/protocol";
import { errorMessage, queryKeys } from "../lib/query";
import type { GetLogsInput, RequestLog } from "../types";
import { LogDetailDrawer } from "../components/LogDetailDrawer";
import { LogRiskBadge } from "../components/LogRiskBadge";
import { Modal, PageTitle, StatusBadge, Toast } from "../components/ui";

const PAGE_SIZE = 50;
const LIVE_BATCH_SIZE = 200;
const MAX_LIVE_BATCHES_PER_SYNC = 5;
const LIVE_SYNC_DELAY_MS = 200;
const LIVE_PROBE_INTERVAL_MS = 5_000;
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

    const lastPage = data.pages[data.pages.length - 1];
    const reachedEnd = lastPage.length < PAGE_SIZE;
    const loadedCapacity = data.pages.length * PAGE_SIZE;
    const merged = [...fresh, ...data.pages.flat()]
        .sort((left, right) => (right.seq ?? 0) - (left.seq ?? 0));
    const retained = reachedEnd ? merged : merged.slice(0, loadedCapacity);
    const pages = Array.from(
        { length: Math.ceil(retained.length / PAGE_SIZE) },
        (_, index) => retained.slice(index * PAGE_SIZE, (index + 1) * PAGE_SIZE),
    );

    return {
        ...data,
        pages,
        pageParams: pages.map((_, index) => index * PAGE_SIZE),
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

function ProtocolBadge({ mode }: { mode: string }) {
    const protocol = getProtocolMeta(mode);
    return <span className={`protocol-badge protocol-${protocol.tone}`}>{protocol.label}</span>;
}

export function LogsPage() {
    const [draftFilters, setDraftFilters] = useState<LogFilterState>(EMPTY_FILTERS);
    const [appliedFilters, setAppliedFilters] = useState<LogFilterState>(EMPTY_FILTERS);
    const [filterError, setFilterError] = useState("");
    const [selectedLog, setSelectedLog] = useState<RequestLog | null>(null);
    const [selectedLogIds, setSelectedLogIds] = useState<Set<string>>(() => new Set());
    const [pendingLiveLogs, setPendingLiveLogs] = useState(0);
    const [exportingFormat, setExportingFormat] = useState<"csv" | "json" | null>(null);
    const [toast, setToast] = useState("");
    const [maintenanceOpen, setMaintenanceOpen] = useState(false);
    const [maintenanceDate, setMaintenanceDate] = useState("");
    const [maintenanceBusy, setMaintenanceBusy] = useState<"before" | "all" | null>(null);
    const queryClient = useQueryClient();
    const logsQueryKey = useMemo(
        () => [...queryKeys.logs, "paged", appliedFilters] as const,
        [appliedFilters],
    );
    const logsQueryKeyRef = useRef(logsQueryKey);
    const filterCursorKey = JSON.stringify(appliedFilters);
    const filterCursorKeyRef = useRef(filterCursorKey);
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
    const selectedLogs = useMemo(
        () => logs.filter((log) => selectedLogIds.has(log.id)),
        [logs, selectedLogIds],
    );
    const allLoadedSelected = logs.length > 0 && selectedLogs.length === logs.length;
    const exportScopeLabel = selectedLogs.length > 0
        ? `导出已选择的 ${selectedLogs.length} 条日志`
        : "导出当前筛选的全部日志";

    const syncLiveLogs = useCallback(async () => {
        const syncKey = filterCursorKey;
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

                if (filterCursorKeyRef.current !== syncKey) {
                    return;
                }

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
    }, [appliedFilters, filterCursorKey, logsQueryKey, queryClient]);

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
        filterCursorKeyRef.current = filterCursorKey;
        syncedSeqRef.current = 0;
        targetSeqRef.current = 0;
        liveSyncAgainRef.current = false;
        setPendingLiveLogs(0);
        if (liveSyncTimerRef.current !== null) {
            clearTimeout(liveSyncTimerRef.current);
            liveSyncTimerRef.current = null;
        }
    }, [filterCursorKey]);

    const probeLiveLogs = useCallback(async () => {
        const probeKey = filterCursorKey;
        if (document.visibilityState === "hidden" || liveSyncingRef.current) {
            return;
        }

        const queryData = queryClient.getQueryData<LogsQueryData>(logsQueryKey);
        if (!queryData) {
            return;
        }

        liveSyncingRef.current = true;
        try {
            let cursor = syncedSeqRef.current;
            let batchCount = 0;
            while (batchCount < MAX_LIVE_BATCHES_PER_SYNC) {
                const rows = await logApi.getAll({
                    ...toGetLogsInput(appliedFilters),
                    limit: LIVE_BATCH_SIZE,
                    after_seq: cursor,
                });
                if (filterCursorKeyRef.current !== probeKey) {
                    return;
                }
                if (rows.length === 0) {
                    break;
                }

                const nextCursor = maxLogSeq(rows);
                if (nextCursor <= cursor) {
                    break;
                }
                queryClient.setQueryData<LogsQueryData>(logsQueryKey, (current) => (
                    current ? mergeLiveLogs(current, rows) : current
                ));
                cursor = nextCursor;
                batchCount += 1;
                if (rows.length < LIVE_BATCH_SIZE) {
                    break;
                }
            }

            syncedSeqRef.current = Math.max(syncedSeqRef.current, cursor);
            targetSeqRef.current = Math.max(targetSeqRef.current, cursor);
            if (syncedSeqRef.current >= targetSeqRef.current) {
                setPendingLiveLogs(0);
            }
        } catch {
            // Tauri 事件不可用或短暂查询失败时，下一个探测周期会继续追赶游标。
        } finally {
            liveSyncingRef.current = false;
            if (liveSyncAgainRef.current || targetSeqRef.current > syncedSeqRef.current) {
                liveSyncAgainRef.current = false;
                queueLiveSyncRef.current();
            }
        }
    }, [appliedFilters, filterCursorKey, logsQueryKey, queryClient]);

    useEffect(() => {
        const loadedSeq = maxLogSeq(logs);
        if (loadedSeq > syncedSeqRef.current) {
            syncedSeqRef.current = loadedSeq;
        }
        if (targetSeqRef.current > syncedSeqRef.current) {
            queueLiveSync();
        }
    }, [logs, queueLiveSync]);

    useTauriEvent<LogChangedEvent>(LOG_CHANGED_EVENT, (payload) => {
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

    useEffect(() => () => {
        if (liveSyncTimerRef.current !== null) {
            clearTimeout(liveSyncTimerRef.current);
            liveSyncTimerRef.current = null;
        }
    }, []);

    useEffect(() => {
        const probe = () => void probeLiveLogs();
        const intervalId = window.setInterval(probe, LIVE_PROBE_INTERVAL_MS);
        const handleVisibilityChange = () => {
            if (document.visibilityState === "visible") {
                probe();
            }
        };
        document.addEventListener("visibilitychange", handleVisibilityChange);
        return () => {
            window.clearInterval(intervalId);
            document.removeEventListener("visibilitychange", handleVisibilityChange);
        };
    }, [probeLiveLogs]);

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
        setSelectedLogIds(new Set());
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
        setSelectedLogIds(new Set());
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

    const toggleLogSelection = (logId: string) => {
        setSelectedLogIds((current) => {
            const next = new Set(current);
            if (next.has(logId)) {
                next.delete(logId);
            } else {
                next.add(logId);
            }
            return next;
        });
    };

    const toggleAllLoadedLogs = () => {
        setSelectedLogIds(allLoadedSelected ? new Set() : new Set(logs.map((log) => log.id)));
    };

    const loadAllMatchingLogs = async () => {
        const collected: RequestLog[] = [];
        let offset = 0;
        while (true) {
            const page = await logApi.getAll(toGetLogsInput(appliedFilters, offset));
            collected.push(...page);
            if (page.length < PAGE_SIZE) {
                break;
            }
            offset += page.length;
        }

        const unique = new Map(collected.map((log) => [log.id, log]));
        return Array.from(unique.values()).sort((left, right) => (right.seq ?? 0) - (left.seq ?? 0));
    };

    const exportLogs = async (format: "csv" | "json") => {
        if (logs.length === 0) {
            return;
        }
        setExportingFormat(format);
        try {
            const exportRows = selectedLogs.length > 0 ? selectedLogs : await loadAllMatchingLogs();
            const content = format === "csv" ? logsToCsv(exportRows) : logsToJson(exportRows);
            const saved = await fileApi.saveExport(content, logExportName(format));
            if (saved) {
                setToast(`已导出 ${exportRows.length} 条日志`);
                window.setTimeout(() => setToast(""), 1800);
            }
        } catch (exportError) {
            setToast(errorMessage(exportError));
            window.setTimeout(() => setToast(""), 2400);
        } finally {
            setExportingFormat(null);
        }
    };

    const resetLogView = async () => {
        syncedSeqRef.current = 0;
        targetSeqRef.current = 0;
        liveSyncAgainRef.current = false;
        setPendingLiveLogs(0);
        setSelectedLog(null);
        setSelectedLogIds(new Set());
        await queryClient.invalidateQueries({ queryKey: queryKeys.logs });
    };

    const deleteBeforeDate = async () => {
        if (!maintenanceDate) {
            setToast("请选择日期");
            window.setTimeout(() => setToast(""), 2200);
            return;
        }
        setMaintenanceBusy("before");
        try {
            const before = new Date(`${maintenanceDate}T00:00:00`).toISOString();
            const deleted = await logApi.deleteBefore(before);
            await resetLogView();
            setMaintenanceOpen(false);
            setToast(`已删除 ${formatNumber(deleted)} 条历史日志`);
            window.setTimeout(() => setToast(""), 2200);
        } catch (error) {
            setToast(errorMessage(error));
            window.setTimeout(() => setToast(""), 2600);
        } finally {
            setMaintenanceBusy(null);
        }
    };

    const deleteAllLogs = async () => {
        setMaintenanceBusy("all");
        try {
            const deleted = await logApi.deleteAll();
            await resetLogView();
            setMaintenanceOpen(false);
            setToast(`已清空 ${formatNumber(deleted)} 条日志`);
            window.setTimeout(() => setToast(""), 2200);
        } catch (error) {
            setToast(errorMessage(error));
            window.setTimeout(() => setToast(""), 2600);
        } finally {
            setMaintenanceBusy(null);
        }
    };

    return (
        <div className="page-enter">
            <PageTitle
                title="请求日志"
                meta={`已加载 ${formatNumber(logs.length)} 条 · 每页 ${PAGE_SIZE} 条`}
                action={(
                    <button type="button" className="button-secondary" onClick={() => setMaintenanceOpen(true)}>
                        <Eraser size={15} />日志管理
                    </button>
                )}
            />

            <form className="toolbar-row log-filter-bar" onSubmit={applyFilters}>
                <label className="search-field log-search">
                    <span className="sr-only">关键词</span>
                    <Search size={16} />
                    <input
                        value={draftFilters.keyword}
                        placeholder="搜索请求 ID、Trace ID、密钥、渠道或模型"
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
                    <div>
                        <p className="text-xs text-muted">已加载 {formatNumber(logs.length)} 条 · 已选择 {formatNumber(selectedLogs.length)} 条</p>
                        <span className="mt-1 flex items-center gap-1.5 text-[10px] text-subtle">
                            <span className="live-dot" />
                            {pendingLiveLogs > 0 ? `同步中 ${formatNumber(pendingLiveLogs)} 条` : `实时更新 · ${loadedPageCount} 页`}
                        </span>
                    </div>
                    <div className="log-export-actions">
                        <Download size={15} className="text-muted" aria-hidden="true" />
                        <button
                            type="button"
                            className="button-secondary"
                            disabled={logs.length === 0 || exportingFormat !== null}
                            aria-label={`${exportScopeLabel}为 CSV`}
                            title={`${exportScopeLabel}为 CSV`}
                            onClick={() => void exportLogs("csv")}
                        >
                            {exportingFormat === "csv" ? <span className="button-spinner" /> : <TableProperties size={15} />}CSV
                        </button>
                        <button
                            type="button"
                            className="button-secondary"
                            disabled={logs.length === 0 || exportingFormat !== null}
                            aria-label={`${exportScopeLabel}为 JSON`}
                            title={`${exportScopeLabel}为 JSON`}
                            onClick={() => void exportLogs("json")}
                        >
                            {exportingFormat === "json" ? <span className="button-spinner" /> : <FileJson2 size={15} />}JSON
                        </button>
                    </div>
                </div>
                <div className="table-scroll">
                    <table className="data-table log-table min-w-[1180px]">
                        <thead>
                            <tr>
                                <th className="log-select-cell">
                                    <input
                                        type="checkbox"
                                        aria-label="选择全部已加载日志"
                                        checked={allLoadedSelected}
                                        onChange={toggleAllLoadedLogs}
                                    />
                                </th>
                                <th>序号</th>
                                <th>时间</th>
                                <th>Trace ID</th>
                                <th>请求类型</th>
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
                                    <td className="log-select-cell">
                                        <input
                                            type="checkbox"
                                            aria-label={`选择请求 ${log.seq ?? log.id}`}
                                            checked={selectedLogIds.has(log.id)}
                                            onClick={(event) => event.stopPropagation()}
                                            onKeyDown={(event) => event.stopPropagation()}
                                            onChange={() => toggleLogSelection(log.id)}
                                        />
                                    </td>
                                    <td className="font-mono text-xs font-semibold text-ink">{log.seq === null ? "--" : `#${log.seq}`}</td>
                                    <td className="font-mono text-[11px] text-muted">{formatDateTime(log.created_at)}</td>
                                    <td>
                                        <code className="log-trace-id" title={log.trace_id ?? undefined}>{log.trace_id ?? "--"}</code>
                                    </td>
                                    <td><ProtocolBadge mode={log.mode} /></td>
                                    <td className="text-xs text-ink">{log.api_key_name ?? "未识别"}</td>
                                    <td className="text-xs text-ink">{log.channel_name ?? "未路由"}</td>
                                    <td>
                                        <span className="model-name">{log.model}</span>
                                        {log.is_stream ? <span className="ml-1.5 text-[10px] text-subtle">STREAM</span> : null}
                                    </td>
                                    <td>
                                        <span className="log-token-value">{formatTokenCount(log.total_tokens)}</span>
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
            {maintenanceOpen ? (
                <Modal
                    title="日志管理"
                    description="清理操作会删除本地请求日志，且无法恢复。"
                    size="sm"
                    onClose={() => { if (!maintenanceBusy) setMaintenanceOpen(false); }}
                    footer={<button type="button" className="button-secondary" disabled={maintenanceBusy !== null} onClick={() => setMaintenanceOpen(false)}>取消</button>}
                >
                    <div className="space-y-5">
                        <label className="field-label">
                            <span>删除此日期之前的日志</span>
                            <input
                                className="field-input"
                                type="date"
                                value={maintenanceDate}
                                onChange={(event) => setMaintenanceDate(event.target.value)}
                            />
                        </label>
                        <button type="button" className="button-secondary w-full justify-center" disabled={maintenanceBusy !== null || !maintenanceDate} onClick={() => void deleteBeforeDate()}>
                            {maintenanceBusy === "before" ? <span className="button-spinner" /> : <Trash2 size={15} />}删除指定日期之前
                        </button>
                        <div className="border-t border-line pt-4">
                            <p className="mb-2 text-xs text-muted">清空全部日志会立即重置实时日志游标。</p>
                            <button type="button" className="button-danger w-full justify-center" disabled={maintenanceBusy !== null} onClick={() => void deleteAllLogs()}>
                                {maintenanceBusy === "all" ? <span className="button-spinner is-inverse" /> : <Trash2 size={15} />}清空全部日志
                            </button>
                        </div>
                    </div>
                </Modal>
            ) : null}
            {toast ? <Toast message={toast} /> : null}
        </div>
    );
}
