import { useDeferredValue, useMemo, useState } from "react";
import {
    CheckCircle2,
    Clock3,
    Eye,
    FilterX,
    Search,
    ShieldAlert,
    TriangleAlert,
} from "lucide-react";
import { formatDateTime, formatDuration, formatNumber } from "../lib/format";
import { useGatewayStore } from "../store/gatewayStore";
import type { RequestLog } from "../types";
import { IconButton, Modal, PageTitle, StatusBadge } from "../components/ui";

function LogStatus({ statusCode }: { statusCode: number }) {
    if (statusCode >= 500) {
        return <StatusBadge status="danger">{statusCode}</StatusBadge>;
    }
    if (statusCode >= 400) {
        return <StatusBadge status="warning">{statusCode}</StatusBadge>;
    }
    return <StatusBadge status="success">{statusCode}</StatusBadge>;
}

function RiskStatus({ log }: { log: RequestLog }) {
    if (log.risk_level === "medium") {
        return <StatusBadge status="warning"><ShieldAlert size={12} />审计</StatusBadge>;
    }
    return <span className="text-xs text-subtle">低风险</span>;
}

export function LogsPage() {
    const logs = useGatewayStore((state) => state.logs);
    const [keyword, setKeyword] = useState("");
    const [channel, setChannel] = useState("全部渠道");
    const [model, setModel] = useState("全部模型");
    const [status, setStatus] = useState("全部状态");
    const [selectedLog, setSelectedLog] = useState<RequestLog | null>(null);
    const deferredKeyword = useDeferredValue(keyword.trim().toLowerCase());
    const channels = useMemo(() => Array.from(new Set(logs.map((log) => log.channel_name).filter(Boolean))) as string[], [logs]);
    const models = useMemo(() => Array.from(new Set(logs.map((log) => log.model))), [logs]);

    const filteredLogs = useMemo(() => logs.filter((log) => {
        const matchesKeyword = !deferredKeyword || [
            log.api_key_name,
            log.channel_name,
            log.model,
            log.error_message,
            log.seq?.toString(),
        ].some((value) => value?.toLowerCase().includes(deferredKeyword));
        const matchesChannel = channel === "全部渠道" || log.channel_name === channel;
        const matchesModel = model === "全部模型" || log.model === model;
        const matchesStatus = status === "全部状态"
            || (status === "成功" && log.status_code < 400)
            || (status === "失败" && log.status_code >= 400);
        return matchesKeyword && matchesChannel && matchesModel && matchesStatus;
    }), [channel, deferredKeyword, logs, model, status]);

    const failures = logs.filter((log) => log.status_code >= 400).length;
    const avgLatency = logs.length > 0
        ? Math.round(logs.reduce((sum, log) => sum + log.duration_ms, 0) / logs.length)
        : 0;
    const successRate = logs.length > 0 ? (1 - failures / logs.length) * 100 : 100;
    const hasFilters = keyword || channel !== "全部渠道" || model !== "全部模型" || status !== "全部状态";

    const resetFilters = () => {
        setKeyword("");
        setChannel("全部渠道");
        setModel("全部模型");
        setStatus("全部状态");
    };

    return (
        <div className="page-enter">
            <PageTitle
                title="请求日志"
                meta={`${formatNumber(logs.length + 128_450)} 条历史记录`}
            />

            <section className="log-stats-strip">
                <div><CheckCircle2 size={17} className="text-accent" /><span>成功率</span><strong>{successRate.toFixed(1)}%</strong></div>
                <div><Clock3 size={17} className="text-data-blue" /><span>平均延迟</span><strong>{formatDuration(avgLatency)}</strong></div>
                <div><TriangleAlert size={17} className="text-warning" /><span>失败请求</span><strong>{failures}</strong></div>
                <div><ShieldAlert size={17} className="text-coral" /><span>安全审计</span><strong>{logs.filter((log) => log.security_action === "audit").length}</strong></div>
            </section>

            <section className="toolbar-row mt-4">
                <label className="search-field log-search">
                    <Search size={16} />
                    <input value={keyword} placeholder="搜索请求 ID、渠道、模型" onChange={(event) => setKeyword(event.target.value)} />
                </label>
                <select className="filter-select" aria-label="按渠道筛选" value={channel} onChange={(event) => setChannel(event.target.value)}>
                    <option>全部渠道</option>
                    {channels.map((item) => <option key={item}>{item}</option>)}
                </select>
                <select className="filter-select" aria-label="按模型筛选" value={model} onChange={(event) => setModel(event.target.value)}>
                    <option>全部模型</option>
                    {models.map((item) => <option key={item}>{item}</option>)}
                </select>
                <select className="filter-select" aria-label="按状态筛选" value={status} onChange={(event) => setStatus(event.target.value)}>
                    <option>全部状态</option>
                    <option>成功</option>
                    <option>失败</option>
                </select>
                {hasFilters ? (
                    <button type="button" className="button-ghost" onClick={resetFilters}><FilterX size={15} />清除</button>
                ) : null}
            </section>

            <section className="panel mt-4 min-w-0">
                <div className="panel-header panel-header-compact">
                    <p className="text-xs text-muted">显示 {filteredLogs.length} 条</p>
                    <span className="flex items-center gap-1.5 text-xs text-muted"><span className="live-dot" />实时记录</span>
                </div>
                <div className="table-scroll">
                    <table className="data-table log-table min-w-[980px]">
                        <thead>
                            <tr>
                                <th>请求</th>
                                <th>时间</th>
                                <th>模型</th>
                                <th>渠道 / 密钥</th>
                                <th>状态</th>
                                <th>Token</th>
                                <th>延迟</th>
                                <th>安全</th>
                                <th aria-label="详情" />
                            </tr>
                        </thead>
                        <tbody>
                            {filteredLogs.map((log) => (
                                <tr key={log.id}>
                                    <td className="font-mono text-xs font-semibold text-ink">#{log.seq}</td>
                                    <td className="font-mono text-[11px] text-muted">{formatDateTime(log.created_at)}</td>
                                    <td>
                                        <span className="model-name">{log.model}</span>
                                        {log.is_stream ? <span className="ml-1.5 text-[10px] text-subtle">STREAM</span> : null}
                                    </td>
                                    <td>
                                        <p className="text-xs text-ink">{log.channel_name}</p>
                                        <p className="mt-0.5 text-[11px] text-subtle">{log.api_key_name}</p>
                                    </td>
                                    <td>
                                        <LogStatus statusCode={log.status_code} />
                                        {log.is_retry ? <span className="ml-1.5 text-[10px] text-warning">重试</span> : null}
                                    </td>
                                    <td className="font-mono text-xs text-ink">{formatNumber(log.total_tokens)}</td>
                                    <td className="font-mono text-xs text-ink">{formatDuration(log.duration_ms)}</td>
                                    <td><RiskStatus log={log} /></td>
                                    <td><IconButton label={`查看请求 ${log.seq}`} onClick={() => setSelectedLog(log)}><Eye size={16} /></IconButton></td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
                {filteredLogs.length === 0 ? (
                    <div className="empty-state">
                        <Search size={22} />
                        <strong>没有匹配的请求</strong>
                        <span>调整筛选条件后再试</span>
                    </div>
                ) : null}
            </section>

            {selectedLog ? (
                <Modal
                    title={`请求 #${selectedLog.seq}`}
                    description={formatDateTime(selectedLog.created_at)}
                    size="lg"
                    onClose={() => setSelectedLog(null)}
                    footer={<button type="button" className="button-secondary" onClick={() => setSelectedLog(null)}>关闭</button>}
                >
                    <div className="log-detail-grid">
                        <div><span>模型</span><strong>{selectedLog.model}</strong></div>
                        <div><span>上游渠道</span><strong>{selectedLog.channel_name}</strong></div>
                        <div><span>状态</span><strong><LogStatus statusCode={selectedLog.status_code} /></strong></div>
                        <div><span>延迟</span><strong>{formatDuration(selectedLog.duration_ms)}</strong></div>
                        <div><span>输入 Token</span><strong>{formatNumber(selectedLog.prompt_tokens)}</strong></div>
                        <div><span>输出 Token</span><strong>{formatNumber(selectedLog.completion_tokens)}</strong></div>
                    </div>
                    {selectedLog.error_message ? (
                        <div className="log-error-box" role="alert">
                            <TriangleAlert size={17} />
                            <span>{selectedLog.error_message}</span>
                        </div>
                    ) : null}
                    <div className="mt-5">
                        <div className="mb-2 flex items-center justify-between">
                            <h3 className="text-sm font-semibold text-ink">请求体</h3>
                            <span className="font-mono text-[10px] text-subtle">application/json</span>
                        </div>
                        <pre className="request-code">{selectedLog.request_body ?? "请求体未保留"}</pre>
                    </div>
                    <div className="mt-5 border-t border-line pt-5">
                        <h3 className="mb-3 text-sm font-semibold text-ink">安全扫描</h3>
                        <div className="flex flex-wrap gap-2">
                            <StatusBadge status={selectedLog.risk_level === "medium" ? "warning" : "success"}>
                                风险分 {selectedLog.risk_score}
                            </StatusBadge>
                            <StatusBadge status={selectedLog.sanitized ? "info" : "neutral"}>
                                {selectedLog.sanitized ? "已脱敏" : "无需脱敏"}
                            </StatusBadge>
                            <StatusBadge status="neutral">{selectedLog.security_action}</StatusBadge>
                        </div>
                    </div>
                </Modal>
            ) : null}
        </div>
    );
}
