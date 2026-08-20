import { type FormEvent, type KeyboardEvent, useId, useState } from "react";
import { Pencil, Plus, RotateCcw, ShieldCheck, Trash2 } from "lucide-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { securityApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type {
    BuiltinRule,
    CreateCustomRuleInput,
    CustomRule,
    SecurityRuleSeverity,
} from "../types";
import { IconButton, Modal, StatusBadge, Toggle } from "./ui";

type RuleView = "builtin" | "custom";

type PendingAction =
    | { kind: "delete-builtin"; rule: BuiltinRule }
    | { kind: "delete-custom"; rule: CustomRule }
    | { kind: "reset-builtin" };

interface SecurityRulesPanelProps {
    onNotice: (message: string) => void;
}

const severityOptions: Array<{ value: SecurityRuleSeverity; label: string }> = [
    { value: "info", label: "提示" },
    { value: "low", label: "低" },
    { value: "medium", label: "中" },
    { value: "high", label: "高" },
    { value: "critical", label: "严重" },
];

const severityLabels = Object.fromEntries(severityOptions.map((option) => [option.value, option.label]));

function severityTone(severity: SecurityRuleSeverity): "danger" | "warning" | "info" | "neutral" {
    if (severity === "critical" || severity === "high") return "danger";
    if (severity === "medium") return "warning";
    if (severity === "low") return "info";
    return "neutral";
}

const emptyCustomRule: CreateCustomRuleInput = {
    rule_type: "blacklist",
    category: "keyword",
    pattern: "",
    severity: "medium",
    action: "warn",
    description: "",
};

export function SecurityRulesPanel({ onNotice }: SecurityRulesPanelProps) {
    const queryClient = useQueryClient();
    const [view, setView] = useState<RuleView>("builtin");
    const builtinTabId = useId();
    const customTabId = useId();
    const builtinPanelId = useId();
    const customPanelId = useId();
    const [busyId, setBusyId] = useState<string | null>(null);
    const [operationError, setOperationError] = useState("");
    const [editingBuiltin, setEditingBuiltin] = useState<BuiltinRule | null>(null);
    const [builtinDraft, setBuiltinDraft] = useState({ title: "", description: "", severity: "medium" as SecurityRuleSeverity });
    const [creatingCustom, setCreatingCustom] = useState(false);
    const [customDraft, setCustomDraft] = useState<CreateCustomRuleInput>(() => ({ ...emptyCustomRule }));
    const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);

    const builtinQuery = useQuery({
        queryKey: queryKeys.securityBuiltinRules,
        queryFn: securityApi.getBuiltinRules,
    });
    const customQuery = useQuery({
        queryKey: queryKeys.securityCustomRules,
        queryFn: securityApi.getCustomRules,
    });

    const refreshBuiltin = () => queryClient.invalidateQueries({ queryKey: queryKeys.securityBuiltinRules });
    const refreshCustom = () => queryClient.invalidateQueries({ queryKey: queryKeys.securityCustomRules });

    const runOperation = async (id: string, operation: () => Promise<unknown>, refresh: () => Promise<unknown>, notice: string) => {
        setBusyId(id);
        setOperationError("");
        try {
            await operation();
            await refresh();
            onNotice(notice);
        } catch (error) {
            setOperationError(errorMessage(error));
            throw error;
        } finally {
            setBusyId(null);
        }
    };

    const toggleBuiltin = async (rule: BuiltinRule, enabled: boolean) => {
        try {
            await runOperation(
                rule.id,
                () => securityApi.updateBuiltinRule(rule.id, { enabled }),
                refreshBuiltin,
                enabled ? "内置规则已启用" : "内置规则已停用",
            );
        } catch {
            // Error is rendered within the panel.
        }
    };

    const toggleCustom = async (rule: CustomRule, enabled: boolean) => {
        try {
            await runOperation(
                rule.id,
                () => securityApi.toggleCustomRule(rule.id, enabled),
                refreshCustom,
                enabled ? "自定义规则已启用" : "自定义规则已停用",
            );
        } catch {
            // Error is rendered within the panel.
        }
    };

    const openBuiltinEditor = (rule: BuiltinRule) => {
        setBuiltinDraft({
            title: rule.title,
            description: rule.description ?? "",
            severity: rule.severity,
        });
        setEditingBuiltin(rule);
        setOperationError("");
    };

    const saveBuiltin = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        if (!editingBuiltin || !builtinDraft.title.trim()) {
            setOperationError("规则标题不能为空");
            return;
        }
        try {
            await runOperation(
                editingBuiltin.id,
                () => securityApi.updateBuiltinRule(editingBuiltin.id, {
                    title: builtinDraft.title.trim(),
                    description: builtinDraft.description.trim(),
                    severity: builtinDraft.severity,
                }),
                refreshBuiltin,
                "内置规则已更新",
            );
            setEditingBuiltin(null);
        } catch {
            // Error is rendered within the modal.
        }
    };

    const createCustom = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        if (!customDraft.pattern.trim()) {
            setOperationError("请填写匹配内容");
            return;
        }
        try {
            await runOperation(
                "create-custom",
                () => securityApi.createCustomRule({
                    ...customDraft,
                    pattern: customDraft.pattern.trim(),
                    description: customDraft.description?.trim() || undefined,
                }),
                refreshCustom,
                "自定义规则已创建",
            );
            setCreatingCustom(false);
            setCustomDraft({ ...emptyCustomRule });
        } catch {
            // Error is rendered within the modal.
        }
    };

    const confirmPendingAction = async () => {
        if (!pendingAction) return;
        try {
            if (pendingAction.kind === "delete-builtin") {
                await runOperation(
                    pendingAction.rule.id,
                    () => securityApi.deleteBuiltinRule(pendingAction.rule.id),
                    refreshBuiltin,
                    "内置规则已删除",
                );
            } else if (pendingAction.kind === "delete-custom") {
                await runOperation(
                    pendingAction.rule.id,
                    () => securityApi.deleteCustomRule(pendingAction.rule.id),
                    refreshCustom,
                    "自定义规则已删除",
                );
            } else {
                await runOperation(
                    "reset-builtin",
                    securityApi.resetBuiltinRules,
                    refreshBuiltin,
                    "内置规则已恢复默认",
                );
            }
            setPendingAction(null);
        } catch {
            // Error is rendered within the confirmation modal.
        }
    };

    const queryError = builtinQuery.error ?? customQuery.error;
    const activeError = operationError || (queryError ? errorMessage(queryError) : "");
    const isLoading = builtinQuery.isPending || customQuery.isPending;
    const handleTabKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.preventDefault();
        const nextView = view === "builtin" ? "custom" : "builtin";
        setView(nextView);
        window.requestAnimationFrame(() => {
            document.getElementById(nextView === "builtin" ? builtinTabId : customTabId)?.focus();
        });
    };

    return (
        <div className="security-rules-panel">
            <div className="security-rule-header">
                <div>
                    <strong>检测规则</strong>
                    <span>管理请求审计使用的内置规则和本地匹配规则</span>
                </div>
                <div className="security-rule-switcher" role="tablist" aria-label="安全规则类型" onKeyDown={handleTabKeyDown}>
                    <button id={builtinTabId} type="button" role="tab" aria-selected={view === "builtin"} aria-controls={builtinPanelId} tabIndex={view === "builtin" ? 0 : -1} className={view === "builtin" ? "is-active" : ""} onClick={() => setView("builtin")}>内置 {builtinQuery.data?.length ?? 0}</button>
                    <button id={customTabId} type="button" role="tab" aria-selected={view === "custom"} aria-controls={customPanelId} tabIndex={view === "custom" ? 0 : -1} className={view === "custom" ? "is-active" : ""} onClick={() => setView("custom")}>自定义 {customQuery.data?.length ?? 0}</button>
                </div>
            </div>

            <div className="security-rule-toolbar">
                <span>{view === "builtin" ? "系统检测器，可调整级别、文案与启用状态" : "按关键词、域名、工具名或路径执行黑白名单匹配"}</span>
                {view === "builtin" ? (
                    <button type="button" className="button-secondary" disabled={busyId !== null} onClick={() => setPendingAction({ kind: "reset-builtin" })}>
                        <RotateCcw size={15} />恢复默认
                    </button>
                ) : (
                    <button type="button" className="button-primary" disabled={busyId !== null} onClick={() => { setOperationError(""); setCreatingCustom(true); }}>
                        <Plus size={15} />新增规则
                    </button>
                )}
            </div>

            {activeError ? <p className="form-error" role="alert">{activeError}</p> : null}

            {isLoading ? (
                <div className="security-rule-empty"><span className="button-spinner" />正在读取规则</div>
            ) : view === "builtin" ? (
                <div id={builtinPanelId} className="security-rule-list" role="tabpanel" aria-labelledby={builtinTabId}>
                    {(builtinQuery.data ?? []).map((rule) => (
                        <article className="security-rule-item" key={rule.id}>
                            <div className="security-rule-main">
                                <div className="security-rule-title">
                                    <strong>{rule.title}</strong>
                                    <StatusBadge status={severityTone(rule.severity)}>{severityLabels[rule.severity]}</StatusBadge>
                                </div>
                                <p>{rule.description || "暂无说明"}</p>
                                <div className="security-rule-meta"><code>{rule.rule_id}</code><span>{rule.category}</span>{rule.toggle_key ? <span>{rule.toggle_key}</span> : null}</div>
                            </div>
                            <div className="security-rule-actions">
                                <Toggle checked={rule.enabled === 1} disabled={busyId !== null} label={`${rule.title}${rule.enabled === 1 ? "已启用" : "已停用"}`} onChange={(enabled) => void toggleBuiltin(rule, enabled)} />
                                <IconButton label={`编辑 ${rule.title}`} disabled={busyId !== null} onClick={() => openBuiltinEditor(rule)}><Pencil size={15} /></IconButton>
                                <IconButton label={`删除 ${rule.title}`} tone="danger" disabled={busyId !== null} onClick={() => setPendingAction({ kind: "delete-builtin", rule })}><Trash2 size={15} /></IconButton>
                            </div>
                        </article>
                    ))}
                </div>
            ) : (
                <div id={customPanelId} className="security-rule-list" role="tabpanel" aria-labelledby={customTabId}>
                    {(customQuery.data ?? []).length === 0 ? (
                        <div className="security-rule-empty"><ShieldCheck size={20} /><span>尚未添加自定义规则</span></div>
                    ) : (customQuery.data ?? []).map((rule) => (
                        <article className="security-rule-item" key={rule.id}>
                            <div className="security-rule-main">
                                <div className="security-rule-title">
                                    <strong>{rule.rule_type === "whitelist" ? "白名单" : "黑名单"} · {rule.category}</strong>
                                    <StatusBadge status={severityTone(rule.severity)}>{severityLabels[rule.severity]}</StatusBadge>
                                </div>
                                <code className="security-rule-pattern">{rule.pattern}</code>
                                <p>{rule.description || "暂无说明"}</p>
                            </div>
                            <div className="security-rule-actions">
                                <Toggle checked={rule.enabled === 1} disabled={busyId !== null} label={`${rule.pattern}${rule.enabled === 1 ? "已启用" : "已停用"}`} onChange={(enabled) => void toggleCustom(rule, enabled)} />
                                <IconButton label={`删除规则 ${rule.pattern}`} tone="danger" disabled={busyId !== null} onClick={() => setPendingAction({ kind: "delete-custom", rule })}><Trash2 size={15} /></IconButton>
                            </div>
                        </article>
                    ))}
                </div>
            )}

            {editingBuiltin ? (
                <Modal title="编辑内置规则" description={editingBuiltin.rule_id} size="sm" onClose={() => { if (!busyId) setEditingBuiltin(null); }} footer={(
                    <><button type="button" className="button-secondary" disabled={busyId !== null} onClick={() => setEditingBuiltin(null)}>取消</button><button type="submit" form="builtin-rule-form" className="button-primary" disabled={busyId !== null}>{busyId ? <span className="button-spinner is-inverse" /> : null}保存规则</button></>
                )}>
                    <form id="builtin-rule-form" className="security-rule-form" onSubmit={saveBuiltin}>
                        <label className="field-label"><span>标题</span><input className="field-input" value={builtinDraft.title} onChange={(event) => setBuiltinDraft((current) => ({ ...current, title: event.target.value }))} autoFocus /></label>
                        <label className="field-label"><span>风险等级</span><select className="field-input" value={builtinDraft.severity} onChange={(event) => setBuiltinDraft((current) => ({ ...current, severity: event.target.value as SecurityRuleSeverity }))}>{severityOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select></label>
                        <label className="field-label"><span>说明</span><textarea className="field-input min-h-24 resize-y" value={builtinDraft.description} onChange={(event) => setBuiltinDraft((current) => ({ ...current, description: event.target.value }))} /></label>
                        {operationError ? <p className="form-error" role="alert">{operationError}</p> : null}
                    </form>
                </Modal>
            ) : null}

            {creatingCustom ? (
                <Modal title="新增自定义规则" description="规则只保存在当前设备" size="sm" onClose={() => { if (!busyId) setCreatingCustom(false); }} footer={(
                    <><button type="button" className="button-secondary" disabled={busyId !== null} onClick={() => setCreatingCustom(false)}>取消</button><button type="submit" form="custom-rule-form" className="button-primary" disabled={busyId !== null}>{busyId ? <span className="button-spinner is-inverse" /> : null}创建规则</button></>
                )}>
                    <form id="custom-rule-form" className="security-rule-form" onSubmit={createCustom}>
                        <div className="form-grid">
                            <label className="field-label"><span>规则类型</span><select className="field-input" value={customDraft.rule_type} onChange={(event) => setCustomDraft((current) => ({ ...current, rule_type: event.target.value }))}><option value="blacklist">黑名单</option><option value="whitelist">白名单</option></select></label>
                            <label className="field-label"><span>匹配分类</span><select className="field-input" value={customDraft.category} onChange={(event) => setCustomDraft((current) => ({ ...current, category: event.target.value }))}><option value="keyword">关键词</option><option value="domain">域名</option><option value="tool">工具名</option><option value="path">文件路径</option></select></label>
                        </div>
                        <label className="field-label"><span>匹配内容</span><input className="field-input font-mono" value={customDraft.pattern} placeholder="例如 example.com 或 ~/.ssh" onChange={(event) => setCustomDraft((current) => ({ ...current, pattern: event.target.value }))} autoFocus /></label>
                        <label className="field-label"><span>风险等级</span><select className="field-input" value={customDraft.severity} onChange={(event) => setCustomDraft((current) => ({ ...current, severity: event.target.value as SecurityRuleSeverity }))}>{severityOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select></label>
                        <label className="field-label"><span>说明（可选）</span><textarea className="field-input min-h-20 resize-y" value={customDraft.description} onChange={(event) => setCustomDraft((current) => ({ ...current, description: event.target.value }))} /></label>
                        {operationError ? <p className="form-error" role="alert">{operationError}</p> : null}
                    </form>
                </Modal>
            ) : null}

            {pendingAction ? (
                <Modal title={pendingAction.kind === "reset-builtin" ? "恢复默认规则" : "删除安全规则"} description={pendingAction.kind === "reset-builtin" ? "所有内置规则的修改都会被覆盖" : "删除后规则将立即停止参与请求检查"} size="sm" onClose={() => { if (!busyId) setPendingAction(null); }} footer={(
                    <><button type="button" className="button-secondary" disabled={busyId !== null} onClick={() => setPendingAction(null)}>取消</button><button type="button" className={pendingAction.kind === "reset-builtin" ? "button-primary" : "button-danger"} disabled={busyId !== null} onClick={() => void confirmPendingAction()}>{busyId ? <span className="button-spinner is-inverse" /> : pendingAction.kind === "reset-builtin" ? <RotateCcw size={15} /> : <Trash2 size={15} />}{pendingAction.kind === "reset-builtin" ? "恢复默认" : "删除规则"}</button></>
                )}>
                    <p className="text-sm leading-6 text-muted">{pendingAction.kind === "reset-builtin" ? "恢复后将重新启用系统默认的 25 条检测规则。" : "已有请求日志不会受影响。"}</p>
                    {operationError ? <p className="form-error mt-3" role="alert">{operationError}</p> : null}
                </Modal>
            ) : null}
        </div>
    );
}
