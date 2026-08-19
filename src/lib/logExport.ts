import type { RequestLog } from "../types";
import { getProtocolMeta } from "./protocol";

const CSV_COLUMNS: ReadonlyArray<{
    header: string;
    value: (log: RequestLog) => string | number | boolean | null;
}> = [
    { header: "id", value: (log) => log.id },
    { header: "seq", value: (log) => log.seq },
    { header: "trace_id", value: (log) => log.trace_id },
    { header: "protocol", value: (log) => getProtocolMeta(log.mode).label },
    { header: "mode", value: (log) => log.mode },
    { header: "created_at", value: (log) => log.created_at },
    { header: "api_key_name", value: (log) => log.api_key_name },
    { header: "channel_name", value: (log) => log.channel_name },
    { header: "model", value: (log) => log.model },
    { header: "upstream_model", value: (log) => log.upstream_model },
    { header: "status_code", value: (log) => log.status_code },
    { header: "total_tokens", value: (log) => log.total_tokens },
    { header: "prompt_tokens", value: (log) => log.prompt_tokens },
    { header: "completion_tokens", value: (log) => log.completion_tokens },
    { header: "duration_ms", value: (log) => log.duration_ms },
    { header: "is_stream", value: (log) => log.is_stream },
    { header: "is_retry", value: (log) => log.is_retry },
    { header: "risk_level", value: (log) => log.risk_level },
    { header: "risk_score", value: (log) => log.risk_score },
    { header: "security_action", value: (log) => log.security_action },
    { header: "sanitized", value: (log) => log.sanitized },
    { header: "error_message", value: (log) => log.error_message },
    { header: "blocked_reason", value: (log) => log.blocked_reason },
    { header: "risk_summary", value: (log) => log.risk_summary },
    { header: "request_body", value: (log) => log.request_body },
    { header: "response_choices", value: (log) => log.response_choices },
];

function csvCell(value: string | number | boolean | null): string {
    if (value === null) {
        return "";
    }
    const text = String(value);
    return /[",\r\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export function logsToCsv(logs: RequestLog[]): string {
    const header = CSV_COLUMNS.map((column) => csvCell(column.header)).join(",");
    const rows = logs.map((log) => CSV_COLUMNS.map((column) => csvCell(column.value(log))).join(","));
    return `\uFEFF${[header, ...rows].join("\r\n")}`;
}

export function logsToJson(logs: RequestLog[]): string {
    return JSON.stringify({
        version: "1.0",
        exported_at: new Date().toISOString(),
        count: logs.length,
        logs: logs.map((log) => ({
            ...log,
            protocol: getProtocolMeta(log.mode).label,
        })),
    }, null, 2);
}

export function logExportName(format: "csv" | "json"): string {
    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    return `crowapi-logs-${timestamp}.${format}`;
}
