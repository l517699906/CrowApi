import { type FormEvent, useMemo, useState } from "react";
import {
    Check,
    Copy,
    KeyRound,
    Plus,
    ShieldCheck,
    Trash2,
    WalletCards,
    XCircle,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { formatCompactNumber, formatDateTime, formatQuota } from "../lib/format";
import { apiKeyApi, channelApi, statsApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type { ApiKey } from "../types";
import { IconButton, Modal, PageTitle, StatusBadge, Toast, Toggle } from "../components/ui";

interface KeyFormState {
    name: string;
    quotaLimit: number;
    modelScope: string;
    channelScope: string;
    expiresAt: string;
}

const initialForm: KeyFormState = {
    name: "",
    quotaLimit: 1_000_000,
    modelScope: "",
    channelScope: "",
    expiresAt: "",
};

export function ApiKeysPage() {
    const queryClient = useQueryClient();
    const { data: apiKeys = [], isPending, error } = useQuery({
        queryKey: queryKeys.apiKeys,
        queryFn: apiKeyApi.getAll,
    });
    const { data: channels = [] } = useQuery({
        queryKey: queryKeys.channels,
        queryFn: channelApi.getAll,
    });
    const { data: dashboard } = useQuery({
        queryKey: queryKeys.dashboard,
        queryFn: statsApi.getDashboard,
    });
    const [createOpen, setCreateOpen] = useState(false);
    const [form, setForm] = useState<KeyFormState>(initialForm);
    const [createdKey, setCreatedKey] = useState<ApiKey | null>(null);
    const [deletingKey, setDeletingKey] = useState<ApiKey | null>(null);
    const [copiedKey, setCopiedKey] = useState<string | null>(null);
    const [toast, setToast] = useState("");

    const models = useMemo(() => Array.from(new Set(channels.flatMap((channel) => channel.models))), [channels]);
    const totalQuota = apiKeys.reduce((sum, key) => sum + Math.max(key.quota_limit, 0), 0);
    const totalUsed = apiKeys.reduce((sum, key) => sum + key.quota_used, 0);
    const totalQuotaPercentage = totalQuota > 0 ? Math.min((totalUsed / totalQuota) * 100, 100) : 0;
    const hasUnlimitedQuota = apiKeys.some((key) => key.quota_limit <= 0);
    const now = Date.now();
    const activeKeys = apiKeys.filter((key) => key.status === 1 && (!key.expires_at || new Date(key.expires_at).getTime() > now));

    const showToast = (message: string) => {
        setToast(message);
        window.setTimeout(() => setToast(""), 1800);
    };

    const refreshKeys = () => queryClient.invalidateQueries({ queryKey: queryKeys.apiKeys });
    const createMutation = useMutation({
        mutationFn: apiKeyApi.create,
        onSuccess: async (apiKey) => {
            setCreatedKey(apiKey);
            await Promise.all([
                refreshKeys(),
                queryClient.invalidateQueries({ queryKey: queryKeys.dashboard }),
            ]);
        },
    });
    const toggleMutation = useMutation({
        mutationFn: (apiKey: ApiKey) => apiKeyApi.update(apiKey.id, apiKey.status === 1 ? 0 : 1),
        onSuccess: refreshKeys,
        onError: (mutationError) => showToast(errorMessage(mutationError)),
    });
    const deleteMutation = useMutation({
        mutationFn: apiKeyApi.delete,
        onSuccess: async () => {
            await Promise.all([
                refreshKeys(),
                queryClient.invalidateQueries({ queryKey: queryKeys.dashboard }),
            ]);
            setDeletingKey(null);
            showToast("密钥已删除");
        },
        onError: (mutationError) => showToast(errorMessage(mutationError)),
    });

    const copyKey = async (apiKey: ApiKey) => {
        try {
            await navigator.clipboard.writeText(apiKey.key);
            setCopiedKey(apiKey.id);
            window.setTimeout(() => setCopiedKey(null), 1500);
        } catch {
            showToast("复制失败，请手动选择密钥");
        }
    };

    const closeCreateDialog = () => {
        setCreateOpen(false);
        setCreatedKey(null);
        setForm(initialForm);
    };

    const submitCreate = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        if (!form.name.trim()) {
            return;
        }

        try {
            await createMutation.mutateAsync({
                name: form.name.trim(),
                quota_limit: Number(form.quotaLimit),
                allowed_models: form.modelScope ? [form.modelScope] : [],
                allowed_channels: form.channelScope ? [form.channelScope] : [],
                expires_at: form.expiresAt ? new Date(`${form.expiresAt}T23:59:59`).toISOString() : null,
            });
        } catch (mutationError) {
            showToast(errorMessage(mutationError));
        }
    };

    return (
        <div className="page-enter">
            <PageTitle
                title="密钥"
                meta={`${activeKeys.length} 个有效密钥`}
                action={(
                    <button type="button" className="button-primary" onClick={() => setCreateOpen(true)}>
                        <Plus size={16} />创建密钥
                    </button>
                )}
            />

            <section className="key-overview">
                <div className="key-overview-main">
                    <span className="overview-icon"><WalletCards size={20} /></span>
                    <div>
                        <p>总配额用量</p>
                        <strong>{formatCompactNumber(totalUsed)} <span>/ {hasUnlimitedQuota || totalQuota === 0 ? "不限" : formatCompactNumber(totalQuota)}</span></strong>
                    </div>
                </div>
                <div className="key-quota-track">
                    <span style={{ width: `${totalQuotaPercentage}%` }} />
                </div>
                <div className="key-overview-meta">
                    <span>{hasUnlimitedQuota || totalQuota === 0 ? "包含不限额密钥" : `已使用 ${Math.round(totalQuotaPercentage)}%`}</span>
                    <span>今日调用 {dashboard?.today_requests ?? 0}</span>
                </div>
            </section>

            <section className="panel mt-4 min-w-0">
                <div className="panel-header">
                    <div>
                        <h2>访问密钥</h2>
                        <p>前缀统一为 sk-crowapi-</p>
                    </div>
                    <StatusBadge status="info"><ShieldCheck size={13} />本地存储</StatusBadge>
                </div>
                <div className="table-scroll">
                    <table className="data-table min-w-[920px]">
                        <thead>
                            <tr>
                                <th>名称</th>
                                <th>密钥</th>
                                <th>范围</th>
                                <th>配额</th>
                                <th>到期时间</th>
                                <th>状态</th>
                                <th className="text-right">操作</th>
                            </tr>
                        </thead>
                        <tbody>
                            {apiKeys.map((apiKey) => {
                                const percentage = apiKey.quota_limit <= 0 ? 0 : Math.min((apiKey.quota_used / apiKey.quota_limit) * 100, 100);
                                const isExpired = apiKey.expires_at
                                    ? new Date(apiKey.expires_at).getTime() <= now
                                    : false;
                                return (
                                    <tr key={apiKey.id}>
                                        <td>
                                            <div className="flex items-center gap-3">
                                                <span className="key-mark"><KeyRound size={16} /></span>
                                                <div>
                                                    <p className="font-medium text-ink">{apiKey.name}</p>
                                                    <p className="mt-0.5 text-[11px] text-subtle">创建于 {formatDateTime(apiKey.created_at)}</p>
                                                </div>
                                            </div>
                                        </td>
                                        <td>
                                            <div className="flex items-center gap-1">
                                                <code className="secret-code">{apiKey.key}</code>
                                            </div>
                                        </td>
                                        <td>
                                            <p className="text-xs text-ink">{apiKey.allowed_models.length > 0 ? apiKey.allowed_models.join(", ") : "全部模型"}</p>
                                            <p className="mt-0.5 text-[11px] text-subtle">
                                                {apiKey.allowed_channels.length > 0
                                                    ? apiKey.allowed_channels.map((id) => channels.find((channel) => channel.id === id)?.name ?? id).join(", ")
                                                    : "全部渠道"}
                                            </p>
                                        </td>
                                        <td>
                                            <div className="w-32">
                                                <div className="mb-1 flex justify-between font-mono text-[10px] text-muted">
                                                    <span>{formatCompactNumber(apiKey.quota_used)}</span>
                                                    <span>{formatQuota(apiKey.quota_limit)}</span>
                                                </div>
                                                <div className="quota-progress">
                                                    <span className={percentage > 85 ? "is-warning" : ""} style={{ width: `${percentage}%` }} />
                                                </div>
                                            </div>
                                        </td>
                                        <td className="text-xs text-muted">
                                            {apiKey.expires_at ? formatDateTime(apiKey.expires_at) : "永不过期"}
                                        </td>
                                        <td>
                                            <div className="flex items-center gap-2">
                                                <Toggle
                                                    checked={apiKey.status === 1}
                                                    label={`${apiKey.status === 1 ? "停用" : "启用"}${apiKey.name}`}
                                                    disabled={toggleMutation.isPending}
                                                    onChange={() => toggleMutation.mutate(apiKey)}
                                                />
                                                <span className="text-xs text-muted">
                                                    {apiKey.status !== 1 ? "停用" : isExpired ? "已过期" : "有效"}
                                                </span>
                                            </div>
                                        </td>
                                        <td>
                                            <div className="flex justify-end">
                                                <IconButton label={`删除 ${apiKey.name}`} tone="danger" onClick={() => setDeletingKey(apiKey)}>
                                                    <Trash2 size={16} />
                                                </IconButton>
                                            </div>
                                        </td>
                                    </tr>
                                );
                            })}
                        </tbody>
                    </table>
                </div>
                {isPending ? (
                    <div className="empty-state"><span className="button-spinner" /><strong>正在读取密钥</strong></div>
                ) : error ? (
                    <div className="empty-state"><XCircle size={22} /><strong>密钥读取失败</strong><span>{errorMessage(error)}</span></div>
                ) : apiKeys.length === 0 ? (
                    <div className="empty-state"><KeyRound size={22} /><strong>尚未创建访问密钥</strong><span>创建后即可调用本地网关</span></div>
                ) : null}
            </section>

            {createOpen ? (
                <Modal
                    title={createdKey ? "密钥已创建" : "创建访问密钥"}
                    description={createdKey ? createdKey.name : "配置调用范围和 Token 配额"}
                    onClose={closeCreateDialog}
                    footer={createdKey ? (
                        <button type="button" className="button-primary" onClick={closeCreateDialog}>完成</button>
                    ) : (
                        <>
                            <button type="button" className="button-secondary" onClick={closeCreateDialog}>取消</button>
                            <button type="submit" form="key-form" className="button-primary" disabled={createMutation.isPending}>
                                {createMutation.isPending ? "创建中..." : "创建密钥"}
                            </button>
                        </>
                    )}
                >
                    {createdKey ? (
                        <div>
                            <div className="created-key-box">
                                <code>{createdKey.key}</code>
                                <IconButton label="复制新密钥" onClick={() => copyKey(createdKey)}>
                                    {copiedKey === createdKey.id ? <Check size={16} /> : <Copy size={16} />}
                                </IconButton>
                            </div>
                            <div className="mt-4 flex items-start gap-3 rounded-md border border-warning/30 bg-warning-soft p-3 text-sm leading-6 text-warning-ink">
                                <ShieldCheck className="mt-0.5 shrink-0" size={17} />
                                <span>请妥善保管此密钥，并仅用于受信任的本地应用。</span>
                            </div>
                        </div>
                    ) : (
                        <form id="key-form" className="space-y-5" onSubmit={submitCreate}>
                            <label className="field-label">
                                <span>密钥名称</span>
                                <input
                                    className="field-input"
                                    required
                                    autoFocus
                                    value={form.name}
                                    placeholder="例如：本地开发"
                                    onChange={(event) => setForm((current) => ({ ...current, name: event.target.value }))}
                                />
                            </label>
                            <label className="field-label">
                                <span>Token 配额</span>
                                <input
                                    className="field-input"
                                    type="number"
                                    min="0"
                                    step="1"
                                    value={form.quotaLimit}
                                    onChange={(event) => setForm((current) => ({ ...current, quotaLimit: Number(event.target.value) }))}
                                />
                                <small>设置为 0 表示不限制</small>
                            </label>
                            <div className="form-grid">
                                <label className="field-label">
                                    <span>允许的模型</span>
                                    <select
                                        className="field-input"
                                        value={form.modelScope}
                                        onChange={(event) => setForm((current) => ({ ...current, modelScope: event.target.value }))}
                                    >
                                        <option value="">全部模型</option>
                                        {models.map((model) => <option key={model} value={model}>{model}</option>)}
                                    </select>
                                </label>
                                <label className="field-label">
                                    <span>允许的渠道</span>
                                    <select
                                        className="field-input"
                                        value={form.channelScope}
                                        onChange={(event) => setForm((current) => ({ ...current, channelScope: event.target.value }))}
                                    >
                                        <option value="">全部渠道</option>
                                        {channels.map((channel) => <option key={channel.id} value={channel.id}>{channel.name}</option>)}
                                    </select>
                                </label>
                            </div>
                            <label className="field-label">
                                <span>到期日期</span>
                                <input
                                    className="field-input"
                                    type="date"
                                    value={form.expiresAt}
                                    min={new Date().toISOString().slice(0, 10)}
                                    onChange={(event) => setForm((current) => ({ ...current, expiresAt: event.target.value }))}
                                />
                                <small>留空表示永不过期</small>
                            </label>
                        </form>
                    )}
                </Modal>
            ) : null}

            {deletingKey ? (
                <Modal
                    title="删除密钥"
                    description={deletingKey.name}
                    size="sm"
                    onClose={() => setDeletingKey(null)}
                    footer={(
                        <>
                            <button type="button" className="button-secondary" onClick={() => setDeletingKey(null)}>取消</button>
                            <button
                                type="button"
                                className="button-danger"
                                onClick={() => {
                                    deleteMutation.mutate(deletingKey.id);
                                }}
                                disabled={deleteMutation.isPending}
                            >
                                <Trash2 size={16} />删除密钥
                            </button>
                        </>
                    )}
                >
                    <p className="text-sm leading-6 text-muted">使用此密钥的客户端将立即无法访问网关。</p>
                </Modal>
            ) : null}

            {toast ? <Toast message={toast} /> : null}
        </div>
    );
}
