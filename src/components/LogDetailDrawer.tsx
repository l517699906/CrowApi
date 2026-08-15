import {
    lazy,
    Suspense,
    useEffect,
    useId,
    useMemo,
    useRef,
} from "react";
import {
    ArrowRight,
    Braces,
    CircleAlert,
    KeyRound,
    MessageSquareText,
    Route,
    ServerCog,
    ShieldAlert,
    SlidersHorizontal,
    X,
} from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { formatDateTime, formatDuration, formatNumber } from "../lib/format";
import { logApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type { RequestLog } from "../types";
import { IconButton, StatusBadge } from "./ui";
import { LogRiskBadge } from "./LogRiskBadge";

const JsonCodeBlock = lazy(() => import("./JsonCodeBlock"));

interface LogDetailDrawerProps {
    log: RequestLog;
    onClose: () => void;
}

interface ConversationEntry {
    content: string;
    role: string;
}

interface ParsedRequestBody {
    code: string;
    value: unknown;
}

const CONVERSATION_FIELDS = new Set(["messages", "input", "system", "instructions"]);

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseRequestBody(rawBody: string | null): ParsedRequestBody {
    if (!rawBody) {
        return { code: "", value: null };
    }

    try {
        const value: unknown = JSON.parse(rawBody);
        return { code: JSON.stringify(value, null, 2), value };
    } catch {
        return { code: rawBody, value: null };
    }
}

function contentToText(content: unknown): string {
    if (typeof content === "string") {
        return content;
    }
    if (content === null || content === undefined) {
        return "（空内容）";
    }
    if (Array.isArray(content)) {
        return content.map((part) => {
            if (typeof part === "string") {
                return part;
            }
            if (isRecord(part)) {
                const text = part.text ?? part.input_text ?? part.output_text;
                if (typeof text === "string") {
                    return text;
                }
                if (typeof part.type === "string") {
                    return `[${part.type}]`;
                }
            }
            return JSON.stringify(part);
        }).join("\n");
    }
    return JSON.stringify(content, null, 2);
}

function extractConversation(value: unknown): ConversationEntry[] {
    if (!isRecord(value)) {
        return [];
    }

    const entries: ConversationEntry[] = [];
    for (const field of ["system", "instructions"] as const) {
        if (typeof value[field] === "string") {
            entries.push({ role: "system", content: value[field] });
        }
    }

    const source = Array.isArray(value.messages)
        ? value.messages
        : Array.isArray(value.input)
            ? value.input
            : typeof value.input === "string"
                ? [value.input]
                : [];

    source.forEach((item) => {
        if (typeof item === "string") {
            entries.push({ role: "user", content: item });
            return;
        }
        if (!isRecord(item)) {
            return;
        }

        const role = typeof item.role === "string" ? item.role : "message";
        const content = item.content ?? item.text ?? item;
        entries.push({ role, content: contentToText(content) });
    });

    return entries;
}

function formatParameterValue(value: unknown): string {
    if (typeof value === "string") {
        return value;
    }
    if (value === null) {
        return "null";
    }
    if (typeof value === "number" || typeof value === "boolean") {
        return String(value);
    }
    return JSON.stringify(value) ?? String(value);
}

function getRoleClass(role: string): string {
    const normalizedRole = role.toLowerCase();
    if (normalizedRole.includes("user")) return "role-user";
    if (normalizedRole.includes("assistant")) return "role-assistant";
    if (normalizedRole.includes("system")) return "role-system";
    if (normalizedRole.includes("tool")) return "role-tool";
    return "role-default";
}

function getStatusTone(statusCode: number): "success" | "warning" | "danger" {
    if (statusCode >= 500) return "danger";
    if (statusCode >= 400) return "warning";
    return "success";
}

export function LogDetailDrawer({ log, onClose }: LogDetailDrawerProps) {
    const titleId = useId();
    const panelRef = useRef<HTMLElement>(null);
    const onCloseRef = useRef(onClose);
    const parsedBody = useMemo(() => parseRequestBody(log.request_body), [log.request_body]);
    const conversation = useMemo(() => extractConversation(parsedBody.value), [parsedBody.value]);
    const requestParameters = useMemo(() => (
        isRecord(parsedBody.value)
            ? Object.entries(parsedBody.value).filter(([key]) => !CONVERSATION_FIELDS.has(key))
            : []
    ), [parsedBody.value]);
    const findingsQuery = useQuery({
        queryKey: queryKeys.logSecurityFindings(log.id),
        queryFn: () => logApi.getSecurityFindings(log.id),
    });

    useEffect(() => {
        onCloseRef.current = onClose;
    }, [onClose]);

    useEffect(() => {
        const previousOverflow = document.body.style.overflow;
        const previousActiveElement = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        document.body.style.overflow = "hidden";

        const handleKeyDown = (event: KeyboardEvent) => {
            if (event.key === "Escape") {
                onCloseRef.current();
                return;
            }
            if (event.key !== "Tab" || !panelRef.current) {
                return;
            }

            const focusableElements = Array.from(panelRef.current.querySelectorAll<HTMLElement>(
                "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])",
            ));
            const firstElement = focusableElements[0];
            const lastElement = focusableElements[focusableElements.length - 1];

            if (event.shiftKey && document.activeElement === firstElement) {
                event.preventDefault();
                lastElement?.focus();
            } else if (!event.shiftKey && document.activeElement === lastElement) {
                event.preventDefault();
                firstElement?.focus();
            }
        };

        window.addEventListener("keydown", handleKeyDown);
        window.requestAnimationFrame(() => panelRef.current?.focus());

        return () => {
            document.body.style.overflow = previousOverflow;
            window.removeEventListener("keydown", handleKeyDown);
            previousActiveElement?.focus();
        };
    }, []);

    const requestLabel = log.seq === null ? log.id.slice(0, 8) : `#${log.seq}`;

    return (
        <div
            className="drawer-backdrop"
            role="presentation"
            onMouseDown={(event) => {
                if (event.target === event.currentTarget) {
                    onClose();
                }
            }}
        >
            <section
                ref={panelRef}
                className="log-detail-drawer"
                role="dialog"
                aria-modal="true"
                aria-labelledby={titleId}
                tabIndex={-1}
            >
                <header className="drawer-header">
                    <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                            <h2 id={titleId}>请求 {requestLabel}</h2>
                            <StatusBadge status={getStatusTone(log.status_code)}>{log.status_code}</StatusBadge>
                            <LogRiskBadge level={log.risk_level} score={log.risk_score} />
                        </div>
                        <p>{formatDateTime(log.created_at)} · {log.mode} · {log.is_stream ? "流式" : "非流式"}</p>
                    </div>
                    <IconButton label="关闭日志详情" onClick={onClose} className="-mr-1 -mt-1">
                        <X size={18} />
                    </IconButton>
                </header>

                <div className="drawer-content">
                    <section className="drawer-section drawer-summary" aria-label="请求概览">
                        <dl className="log-detail-summary">
                            <div><dt>模型</dt><dd>{log.model}</dd></div>
                            <div><dt>上游模型</dt><dd>{log.upstream_model ?? "未路由"}</dd></div>
                            <div><dt>密钥</dt><dd>{log.api_key_name ?? "未识别"}</dd></div>
                            <div><dt>渠道</dt><dd>{log.channel_name ?? "未路由"}</dd></div>
                            <div><dt>Token</dt><dd>{formatNumber(log.prompt_tokens)} + {formatNumber(log.completion_tokens)}</dd></div>
                            <div><dt>延迟</dt><dd>{formatDuration(log.duration_ms)}</dd></div>
                        </dl>
                        {log.error_message || log.blocked_reason ? (
                            <div className="log-error-box" role="alert">
                                <CircleAlert size={17} />
                                <span>{log.blocked_reason ?? log.error_message}</span>
                            </div>
                        ) : null}
                    </section>

                    <section className="drawer-section">
                        <div className="drawer-section-heading">
                            <MessageSquareText size={16} />
                            <div><h3>对话构成</h3><p>{conversation.length} 条消息</p></div>
                        </div>
                        {conversation.length > 0 ? (
                            <ol className="conversation-list">
                                {conversation.map((message, index) => (
                                    <li key={`${message.role}-${index}`}>
                                        <span className={`conversation-role ${getRoleClass(message.role)}`}>{message.role}</span>
                                        <p>{message.content}</p>
                                    </li>
                                ))}
                            </ol>
                        ) : (
                            <p className="drawer-empty-note">请求体中没有可识别的对话消息</p>
                        )}
                    </section>

                    <section className="drawer-section">
                        <div className="drawer-section-heading">
                            <SlidersHorizontal size={16} />
                            <div><h3>请求参数</h3><p>不含对话正文</p></div>
                        </div>
                        {requestParameters.length > 0 ? (
                            <dl className="request-parameter-list">
                                {requestParameters.map(([key, value]) => (
                                    <div key={key}>
                                        <dt>{key}</dt>
                                        <dd>{formatParameterValue(value)}</dd>
                                    </div>
                                ))}
                            </dl>
                        ) : (
                            <p className="drawer-empty-note">没有可展示的请求参数</p>
                        )}
                    </section>

                    <section className="drawer-section">
                        <div className="drawer-section-heading">
                            <Route size={16} />
                            <div><h3>网关路由</h3><p>{log.is_retry ? "发生过重试" : "首次路由"}</p></div>
                        </div>
                        <div className="gateway-route-track">
                            <div className="gateway-route-node">
                                <KeyRound size={16} />
                                <span>调用密钥</span>
                                <strong>{log.api_key_name ?? "未识别"}</strong>
                            </div>
                            <ArrowRight className="gateway-route-arrow" size={17} />
                            <div className="gateway-route-node is-gateway">
                                <Braces size={16} />
                                <span>CrowAPI 网关</span>
                                <strong>{log.model}</strong>
                            </div>
                            <ArrowRight className="gateway-route-arrow" size={17} />
                            <div className="gateway-route-node">
                                <ServerCog size={16} />
                                <span>{log.channel_name ?? "未路由"}</span>
                                <strong>{log.upstream_model ?? log.model}</strong>
                            </div>
                        </div>
                    </section>

                    <section className="drawer-section">
                        <div className="drawer-section-heading">
                            <Braces size={16} />
                            <div><h3>原始 JSON</h3><p>application/json</p></div>
                        </div>
                        {parsedBody.code ? (
                            <Suspense fallback={<pre className="request-code">{parsedBody.code}</pre>}>
                                <JsonCodeBlock code={parsedBody.code} />
                            </Suspense>
                        ) : (
                            <p className="drawer-empty-note">请求体未保留</p>
                        )}
                    </section>

                    <section className="drawer-section">
                        <div className="drawer-section-heading">
                            <ShieldAlert size={16} />
                            <div><h3>安全审计 Findings</h3><p>{log.risk_summary ?? "按规则逐项记录"}</p></div>
                        </div>
                        <div className="flex flex-wrap gap-2">
                            <LogRiskBadge level={log.risk_level} score={log.risk_score} />
                            <StatusBadge status={log.sanitized ? "info" : "neutral"}>
                                {log.sanitized ? "已脱敏" : "未脱敏"}
                            </StatusBadge>
                            <StatusBadge status="neutral">{log.security_action}</StatusBadge>
                        </div>
                        {findingsQuery.isPending ? (
                            <div className="drawer-loading"><span className="button-spinner" />正在读取审计明细</div>
                        ) : findingsQuery.error ? (
                            <div className="drawer-inline-error"><CircleAlert size={15} />{errorMessage(findingsQuery.error)}</div>
                        ) : findingsQuery.data && findingsQuery.data.length > 0 ? (
                            <ul className="finding-list">
                                {findingsQuery.data.map((finding) => (
                                    <li key={finding.id}>
                                        <div className="finding-heading">
                                            <LogRiskBadge level={finding.severity} />
                                            <span className="finding-phase">{finding.phase}</span>
                                            <strong>{finding.title}</strong>
                                        </div>
                                        {finding.description ? <p>{finding.description}</p> : null}
                                        <dl>
                                            <div><dt>规则</dt><dd>{finding.rule_id}</dd></div>
                                            {finding.location ? <div><dt>位置</dt><dd>{finding.location}</dd></div> : null}
                                            {finding.evidence_masked ? <div><dt>证据</dt><dd>{finding.evidence_masked}</dd></div> : null}
                                            {finding.action ? <div><dt>动作</dt><dd>{finding.action}</dd></div> : null}
                                        </dl>
                                    </li>
                                ))}
                            </ul>
                        ) : (
                            <p className="drawer-empty-note">没有安全审计发现</p>
                        )}
                    </section>
                </div>
            </section>
        </div>
    );
}
