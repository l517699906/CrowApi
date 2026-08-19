import { type FormEvent, useDeferredValue, useMemo, useState } from "react";
import {
    CheckCircle2,
    Clock3,
    FlaskConical,
    GripVertical,
    Layers3,
    Pencil,
    Plus,
    Radio,
    Search,
    Trash2,
    XCircle,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { PROVIDERS, PROVIDER_DEFAULTS, providerLabel } from "../config/providers";
import { formatDateTime, formatDuration } from "../lib/format";
import { channelApi, statsApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import { localDayStatsInput } from "../lib/statistics";
import type { Channel, ChannelType, CreateChannelInput, UpdateChannelInput } from "../types";
import {
    IconButton,
    Modal,
    PageTitle,
    ProviderMark,
    StatusBadge,
    Toast,
    Toggle,
} from "../components/ui";

interface ChannelFormState {
    name: string;
    type: ChannelType;
    baseUrl: string;
    apiKey: string;
    models: string;
    priority: number;
    weight: number;
}

function getInitialForm(channel?: Channel): ChannelFormState {
    if (channel) {
        return {
            name: channel.name,
            type: channel.type,
            baseUrl: channel.base_url,
            apiKey: "",
            models: channel.models.join(", "),
            priority: channel.priority,
            weight: channel.weight,
        };
    }

    return {
        name: "",
        type: "openai",
        baseUrl: PROVIDER_DEFAULTS.openai.baseUrl,
        apiKey: "",
        models: PROVIDER_DEFAULTS.openai.models,
        priority: 10,
        weight: 100,
    };
}

interface ChannelDialogProps {
    channel?: Channel;
    onClose: () => void;
}

function ChannelDialog({ channel, onClose }: ChannelDialogProps) {
    const queryClient = useQueryClient();
    const [form, setForm] = useState<ChannelFormState>(() => getInitialForm(channel));
    const [error, setError] = useState("");
    const saveMutation = useMutation({
        mutationFn: (input: CreateChannelInput | UpdateChannelInput) => (
            "id" in input ? channelApi.update(input) : channelApi.create(input)
        ),
        onSuccess: async () => {
            await Promise.all([
                queryClient.invalidateQueries({ queryKey: queryKeys.channels }),
                queryClient.invalidateQueries({ queryKey: queryKeys.dashboard }),
            ]);
            onClose();
        },
    });

    const submit = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        const models = form.models.split(",").map((model) => model.trim()).filter(Boolean);
        if (!form.name.trim() || !form.baseUrl.trim() || models.length === 0) {
            setError("请填写渠道名称、API 地址和至少一个模型");
            return;
        }
        if (!channel && form.type !== "custom" && !form.apiKey.trim()) {
            setError("请填写上游 API Key");
            return;
        }

        const input: CreateChannelInput = {
            name: form.name.trim(),
            type: form.type,
            base_url: form.baseUrl.trim(),
            api_key: form.apiKey.trim(),
            models,
            priority: Number(form.priority),
            weight: Number(form.weight),
        };

        setError("");
        try {
            if (channel) {
                const updateInput: UpdateChannelInput = {
                    id: channel.id,
                    name: input.name,
                    type: input.type,
                    base_url: input.base_url,
                    models: input.models,
                    priority: input.priority,
                    weight: input.weight,
                    ...(input.api_key ? { api_key: input.api_key } : {}),
                };
                await saveMutation.mutateAsync(updateInput);
            } else {
                await saveMutation.mutateAsync(input);
            }
        } catch (mutationError) {
            setError(errorMessage(mutationError));
        }
    };

    return (
        <Modal
            title={channel ? "编辑渠道" : "添加渠道"}
            description={channel ? channel.name : "配置一个新的上游模型服务"}
            onClose={onClose}
            size="lg"
            footer={(
                <>
                    <button type="button" className="button-secondary" onClick={onClose}>取消</button>
                    <button type="submit" form="channel-form" className="button-primary" disabled={saveMutation.isPending}>
                        {saveMutation.isPending ? "保存中..." : channel ? "保存更改" : "添加渠道"}
                    </button>
                </>
            )}
        >
            <form id="channel-form" className="space-y-5" onSubmit={submit}>
                <div className="form-grid">
                    <label className="field-label">
                        <span>渠道名称</span>
                        <input
                            className="field-input"
                            value={form.name}
                            placeholder="例如：OpenAI 主线路"
                            autoFocus
                            onChange={(event) => setForm((current) => ({ ...current, name: event.target.value }))}
                        />
                    </label>
                    <label className="field-label">
                        <span>渠道类型</span>
                        <select
                            className="field-input"
                            value={form.type}
                            onChange={(event) => {
                                const type = event.target.value as ChannelType;
                                setForm((current) => ({
                                    ...current,
                                    type,
                                    baseUrl: PROVIDER_DEFAULTS[type].baseUrl,
                                    models: PROVIDER_DEFAULTS[type].models,
                                }));
                            }}
                        >
                            {PROVIDERS.map((provider) => <option key={provider.value} value={provider.value}>{provider.label}</option>)}
                        </select>
                    </label>
                </div>

                <label className="field-label">
                    <span>API 地址</span>
                    <input
                        className="field-input font-mono"
                        value={form.baseUrl}
                        placeholder="https://api.example.com/v1"
                        onChange={(event) => setForm((current) => ({ ...current, baseUrl: event.target.value }))}
                    />
                </label>

                <label className="field-label">
                    <span>上游 API Key</span>
                    <input
                        className="field-input font-mono"
                        type="password"
                        value={form.apiKey}
                        placeholder={channel ? "已保存，留空则不修改" : "sk-..."}
                        autoComplete="off"
                        onChange={(event) => setForm((current) => ({ ...current, apiKey: event.target.value }))}
                    />
                </label>

                <label className="field-label">
                    <span>模型列表</span>
                    <input
                        className="field-input font-mono"
                        value={form.models}
                        placeholder="model-a, model-b"
                        onChange={(event) => setForm((current) => ({ ...current, models: event.target.value }))}
                    />
                </label>

                <div className="form-grid">
                    <label className="field-label">
                        <span>优先级</span>
                        <input
                            className="field-input"
                            type="number"
                            min="1"
                            max="100"
                            value={form.priority}
                            onChange={(event) => setForm((current) => ({ ...current, priority: Number(event.target.value) }))}
                        />
                    </label>
                    <label className="field-label">
                        <span>路由权重</span>
                        <input
                            className="field-input"
                            type="number"
                            min="1"
                            max="100"
                            value={form.weight}
                            onChange={(event) => setForm((current) => ({ ...current, weight: Number(event.target.value) }))}
                        />
                    </label>
                </div>
                {error ? <p className="form-error" role="alert">{error}</p> : null}
            </form>
        </Modal>
    );
}

export function ChannelsPage() {
    const queryClient = useQueryClient();
    const { data: channels = [], isPending, error } = useQuery({
        queryKey: queryKeys.channels,
        queryFn: channelApi.getAll,
    });
    const { data: dashboard } = useQuery({
        queryKey: queryKeys.dashboard,
        queryFn: () => statsApi.getDashboard(localDayStatsInput()),
        refetchInterval: 5_000,
    });
    const [query, setQuery] = useState("");
    const [typeFilter, setTypeFilter] = useState("全部");
    const [editingChannel, setEditingChannel] = useState<Channel | null | undefined>(undefined);
    const [deletingChannel, setDeletingChannel] = useState<Channel | null>(null);
    const [testingId, setTestingId] = useState<string | null>(null);
    const [draggedId, setDraggedId] = useState<string | null>(null);
    const [dragOverId, setDragOverId] = useState<string | null>(null);
    const [toast, setToast] = useState("");
    const deferredQuery = useDeferredValue(query.trim().toLowerCase());

    const filteredChannels = useMemo(() => channels.filter((channel) => {
        const matchesType = typeFilter === "全部" || channel.type === typeFilter;
        const matchesQuery = !deferredQuery
            || channel.name.toLowerCase().includes(deferredQuery)
            || channel.models.some((model) => model.toLowerCase().includes(deferredQuery));
        return matchesType && matchesQuery;
    }), [channels, deferredQuery, typeFilter]);

    const showToast = (message: string) => {
        setToast(message);
        window.setTimeout(() => setToast(""), 1800);
    };

    const refreshChannels = () => queryClient.invalidateQueries({ queryKey: queryKeys.channels });
    const toggleMutation = useMutation({
        mutationFn: (channel: Channel) => channelApi.toggle(channel.id, channel.status === 1 ? 0 : 1),
        onSuccess: refreshChannels,
        onError: (mutationError) => showToast(errorMessage(mutationError)),
    });
    const deleteMutation = useMutation({
        mutationFn: channelApi.delete,
        onSuccess: async () => {
            await Promise.all([
                refreshChannels(),
                queryClient.invalidateQueries({ queryKey: queryKeys.dashboard }),
            ]);
            setDeletingChannel(null);
            showToast("渠道已删除");
        },
        onError: (mutationError) => showToast(errorMessage(mutationError)),
    });
    const testMutation = useMutation({
        mutationFn: channelApi.test,
        onSuccess: async (result, channelId) => {
            await refreshChannels();
            const channel = channels.find((item) => item.id === channelId);
            showToast(`${channel?.name ?? "渠道"}: ${result.message} (${result.latency_ms} ms)`);
        },
        onError: (mutationError) => showToast(errorMessage(mutationError)),
        onSettled: () => setTestingId(null),
    });
    const reorderMutation = useMutation<
        void,
        unknown,
        Channel[],
        { previousChannels: Channel[] | undefined }
    >({
        mutationFn: (nextChannels) => channelApi.reorder({
            ordered_ids: nextChannels.map((channel) => channel.id),
        }),
        onMutate: async (nextChannels) => {
            await queryClient.cancelQueries({ queryKey: queryKeys.channels });
            const previousChannels = queryClient.getQueryData<Channel[]>(queryKeys.channels);
            queryClient.setQueryData(queryKeys.channels, nextChannels);
            return { previousChannels };
        },
        onSuccess: () => showToast("渠道优先级已更新"),
        onError: (mutationError, _nextChannels, context) => {
            if (context?.previousChannels) {
                queryClient.setQueryData(queryKeys.channels, context.previousChannels);
            }
            showToast(`排序失败，已恢复原顺序：${errorMessage(mutationError)}`);
        },
        onSettled: async () => {
            await Promise.all([
                queryClient.invalidateQueries({ queryKey: queryKeys.channels }),
                queryClient.invalidateQueries({ queryKey: queryKeys.dashboard }),
            ]);
        },
    });

    const testChannel = async (channel: Channel) => {
        setTestingId(channel.id);
        await testMutation.mutateAsync(channel.id).catch(() => undefined);
    };

    const reorderVisibleChannels = (sourceId: string, targetId: string) => {
        if (sourceId === targetId || reorderMutation.isPending) {
            return;
        }

        const visibleIds = filteredChannels.map((channel) => channel.id);
        const sourceIndex = visibleIds.indexOf(sourceId);
        const targetIndex = visibleIds.indexOf(targetId);
        if (sourceIndex < 0 || targetIndex < 0) {
            return;
        }

        const reorderedVisibleIds = [...visibleIds];
        const [movedId] = reorderedVisibleIds.splice(sourceIndex, 1);
        if (!movedId) {
            return;
        }
        reorderedVisibleIds.splice(targetIndex, 0, movedId);
        const visibleIdSet = new Set(visibleIds);
        let visibleIndex = 0;
        const channelById = new Map(channels.map((channel) => [channel.id, channel]));
        const reorderedChannels = channels.map((channel) => (
            visibleIdSet.has(channel.id)
                ? channelById.get(reorderedVisibleIds[visibleIndex++]) ?? channel
                : channel
        )).map((channel, index, orderedChannels) => ({
            ...channel,
            priority: orderedChannels.length - index,
        }));

        reorderMutation.mutate(reorderedChannels);
    };

    const moveChannelByKeyboard = (channelId: string, direction: -1 | 1) => {
        const currentIndex = filteredChannels.findIndex((channel) => channel.id === channelId);
        const target = filteredChannels[currentIndex + direction];
        if (currentIndex < 0 || !target) {
            return;
        }
        reorderVisibleChannels(channelId, target.id);
    };

    const activeCount = channels.filter((channel) => channel.status === 1).length;
    const failedCount = channels.filter((channel) => channel.last_test_ok === 0).length;

    return (
        <div className="page-enter">
            <PageTitle
                title="渠道"
                meta={`${activeCount} 条启用 · ${channels.length} 条已配置`}
                action={(
                    <button type="button" className="button-primary" onClick={() => setEditingChannel(null)}>
                        <Plus size={16} />添加渠道
                    </button>
                )}
            />

            <section className="channel-summary-grid" aria-label="渠道统计摘要">
                <div className="summary-tile"><Layers3 size={18} className="text-data-blue" /><div><span>总渠道</span><strong>{channels.length}</strong></div></div>
                <div className="summary-tile"><CheckCircle2 size={18} className="text-accent" /><div><span>活跃渠道</span><strong>{activeCount}</strong></div></div>
                <div className="summary-tile"><XCircle size={18} className="text-danger" /><div><span>失败渠道</span><strong>{failedCount}</strong></div></div>
                <div className="summary-tile"><Clock3 size={18} className="text-warning" /><div><span>平均延迟</span><strong>{formatDuration(Math.round(dashboard?.avg_latency_ms ?? 0))}</strong></div></div>
            </section>

            <section className="toolbar-row mt-4">
                <label className="search-field">
                    <Search size={16} />
                    <input value={query} placeholder="搜索渠道或模型" onChange={(event) => setQuery(event.target.value)} />
                </label>
                <label className="filter-select-wrap">
                    <span className="sr-only">渠道类型</span>
                    <select className="filter-select" value={typeFilter} onChange={(event) => setTypeFilter(event.target.value)}>
                        <option>全部</option>
                        {PROVIDERS.map((provider) => <option key={provider.value} value={provider.value}>{provider.label}</option>)}
                    </select>
                </label>
                <div className="ml-auto flex items-center gap-2 text-xs text-muted" aria-live="polite">
                    {reorderMutation.isPending ? <span className="button-spinner" /> : <GripVertical size={14} />}
                    {reorderMutation.isPending ? "正在保存优先级" : "拖拽卡片调整分发顺序"}
                </div>
            </section>

            <section className="channel-card-grid mt-4" aria-label="渠道优先级列表">
                {filteredChannels.map((channel) => (
                    <article
                        key={channel.id}
                        className={`channel-card ${draggedId === channel.id ? "is-dragging" : ""} ${dragOverId === channel.id ? "is-drag-over" : ""}`}
                        draggable={!reorderMutation.isPending}
                        onDragStart={(event) => {
                            setDraggedId(channel.id);
                            event.dataTransfer.effectAllowed = "move";
                            event.dataTransfer.setData("text/plain", channel.id);
                        }}
                        onDragEnd={() => {
                            setDraggedId(null);
                            setDragOverId(null);
                        }}
                        onDragOver={(event) => {
                            event.preventDefault();
                            event.dataTransfer.dropEffect = "move";
                            setDragOverId(channel.id);
                        }}
                        onDrop={(event) => {
                            event.preventDefault();
                            const sourceId = event.dataTransfer.getData("text/plain") || draggedId;
                            setDraggedId(null);
                            setDragOverId(null);
                            if (sourceId) {
                                reorderVisibleChannels(sourceId, channel.id);
                            }
                        }}
                    >
                        <div className="channel-card-rank">
                            <button
                                type="button"
                                className="channel-drag-handle"
                                aria-label={`调整 ${channel.name} 的优先级，当前第 ${channels.findIndex((item) => item.id === channel.id) + 1} 位`}
                                disabled={reorderMutation.isPending}
                                onKeyDown={(event) => {
                                    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
                                        event.preventDefault();
                                        moveChannelByKeyboard(channel.id, event.key === "ArrowUp" ? -1 : 1);
                                    }
                                }}
                            >
                                <GripVertical size={17} />
                            </button>
                            <span>{String(channels.findIndex((item) => item.id === channel.id) + 1).padStart(2, "0")}</span>
                        </div>
                        <div className="channel-card-main">
                            <div className="channel-card-heading">
                                <ProviderMark type={channel.type} />
                                <div className="min-w-0">
                                    <h2>{channel.name}</h2>
                                    <p>{providerLabel(channel.type)}</p>
                                </div>
                                <div className="channel-card-status">
                                    {channel.last_test_ok === 1 ? (
                                        <StatusBadge status="success" dot>正常</StatusBadge>
                                    ) : channel.last_test_ok === 0 ? (
                                        <StatusBadge status="danger" dot>异常</StatusBadge>
                                    ) : (
                                        <StatusBadge status="neutral">未测试</StatusBadge>
                                    )}
                                </div>
                            </div>
                            <code className="channel-card-url" title={channel.base_url}>{channel.base_url}</code>
                            <div className="channel-card-models">
                                {channel.models.slice(0, 3).map((model) => <span className="model-name" key={model}>{model}</span>)}
                                {channel.models.length > 3 ? <span className="model-more">+{channel.models.length - 3}</span> : null}
                            </div>
                        </div>
                        <dl className="channel-card-routing">
                            <div><dt>优先级</dt><dd>P{channel.priority}</dd></div>
                            <div><dt>权重</dt><dd>W{channel.weight}</dd></div>
                            <div><dt>最近测试</dt><dd>{channel.last_test_at ? formatDateTime(channel.last_test_at) : "--"}</dd></div>
                        </dl>
                        <div className="channel-card-actions">
                            <Toggle
                                checked={channel.status === 1}
                                label={`${channel.status === 1 ? "停用" : "启用"}${channel.name}`}
                                disabled={toggleMutation.isPending}
                                onChange={() => toggleMutation.mutate(channel)}
                            />
                            <IconButton label={`测试 ${channel.name}`} disabled={testingId === channel.id} onClick={() => testChannel(channel)}>
                                {testingId === channel.id ? <span className="button-spinner" /> : <FlaskConical size={16} />}
                            </IconButton>
                            <IconButton label={`编辑 ${channel.name}`} onClick={() => setEditingChannel(channel)}><Pencil size={16} /></IconButton>
                            <IconButton label={`删除 ${channel.name}`} tone="danger" onClick={() => setDeletingChannel(channel)}><Trash2 size={16} /></IconButton>
                        </div>
                    </article>
                ))}
                {isPending ? (
                    <div className="empty-state"><span className="button-spinner" /><strong>正在读取渠道</strong></div>
                ) : error ? (
                    <div className="empty-state"><XCircle size={22} /><strong>渠道读取失败</strong><span>{errorMessage(error)}</span></div>
                ) : filteredChannels.length === 0 ? (
                    <div className="empty-state">
                        <Radio size={22} />
                        <strong>{channels.length === 0 ? "尚未配置渠道" : "没有匹配的渠道"}</strong>
                        <span>{channels.length === 0 ? "添加上游渠道后即可测试连接" : "调整搜索关键词或渠道类型"}</span>
                    </div>
                ) : null}
            </section>

            {editingChannel !== undefined ? (
                <ChannelDialog
                    key={editingChannel?.id ?? "new"}
                    channel={editingChannel ?? undefined}
                    onClose={() => setEditingChannel(undefined)}
                />
            ) : null}

            {deletingChannel ? (
                <Modal
                    title="删除渠道"
                    description={deletingChannel.name}
                    size="sm"
                    onClose={() => setDeletingChannel(null)}
                    footer={(
                        <>
                            <button type="button" className="button-secondary" onClick={() => setDeletingChannel(null)}>取消</button>
                            <button
                                type="button"
                                className="button-danger"
                                onClick={() => {
                                    deleteMutation.mutate(deletingChannel.id);
                                }}
                                disabled={deleteMutation.isPending}
                            >
                                <Trash2 size={16} />删除渠道
                            </button>
                        </>
                    )}
                >
                    <p className="text-sm leading-6 text-muted">删除后该渠道将立即退出路由，已有请求日志不会被删除。</p>
                </Modal>
            ) : null}

            {toast ? <Toast message={toast} /> : null}
        </div>
    );
}
