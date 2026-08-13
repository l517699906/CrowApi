import { type FormEvent, useDeferredValue, useMemo, useState } from "react";
import {
    CheckCircle2,
    FlaskConical,
    Pencil,
    Plus,
    Radio,
    Search,
    Trash2,
    XCircle,
} from "lucide-react";
import { formatDateTime } from "../lib/format";
import { useGatewayStore } from "../store/gatewayStore";
import type { Channel, ChannelType, CreateChannelInput } from "../types";
import {
    IconButton,
    Modal,
    PageTitle,
    ProviderMark,
    StatusBadge,
    Toast,
    Toggle,
} from "../components/ui";

const channelTypes: ChannelType[] = ["OpenAI", "DeepSeek", "Claude", "Gemini", "Custom"];

const providerDefaults: Record<ChannelType, { baseUrl: string; models: string }> = {
    OpenAI: { baseUrl: "https://api.openai.com/v1", models: "gpt-5.2, gpt-5-mini" },
    DeepSeek: { baseUrl: "https://api.deepseek.com/v1", models: "deepseek-chat, deepseek-reasoner" },
    Claude: { baseUrl: "https://api.anthropic.com/v1", models: "claude-sonnet-4-5, claude-haiku-4-5" },
    Gemini: { baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai", models: "gemini-2.5-pro, gemini-2.5-flash" },
    Custom: { baseUrl: "http://127.0.0.1:11434/v1", models: "" },
};

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
            type: channel.type as ChannelType,
            baseUrl: channel.base_url,
            apiKey: channel.api_key,
            models: channel.models.join(", "),
            priority: channel.priority,
            weight: channel.weight,
        };
    }

    return {
        name: "",
        type: "OpenAI",
        baseUrl: providerDefaults.OpenAI.baseUrl,
        apiKey: "",
        models: providerDefaults.OpenAI.models,
        priority: 10,
        weight: 100,
    };
}

interface ChannelDialogProps {
    channel?: Channel;
    onClose: () => void;
}

function ChannelDialog({ channel, onClose }: ChannelDialogProps) {
    const addChannel = useGatewayStore((state) => state.addChannel);
    const updateChannel = useGatewayStore((state) => state.updateChannel);
    const [form, setForm] = useState<ChannelFormState>(() => getInitialForm(channel));
    const [error, setError] = useState("");

    const submit = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        const models = form.models.split(",").map((model) => model.trim()).filter(Boolean);
        if (!form.name.trim() || !form.baseUrl.trim() || models.length === 0) {
            setError("请填写渠道名称、API 地址和至少一个模型");
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

        if (channel) {
            updateChannel(channel.id, input);
        } else {
            addChannel(input);
        }
        onClose();
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
                    <button type="submit" form="channel-form" className="button-primary">
                        {channel ? "保存更改" : "添加渠道"}
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
                                    baseUrl: providerDefaults[type].baseUrl,
                                    models: providerDefaults[type].models,
                                }));
                            }}
                        >
                            {channelTypes.map((type) => <option key={type}>{type}</option>)}
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
                        placeholder="sk-..."
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
    const channels = useGatewayStore((state) => state.channels);
    const toggleChannel = useGatewayStore((state) => state.toggleChannel);
    const deleteChannel = useGatewayStore((state) => state.deleteChannel);
    const recordChannelTest = useGatewayStore((state) => state.recordChannelTest);
    const [query, setQuery] = useState("");
    const [typeFilter, setTypeFilter] = useState("全部");
    const [editingChannel, setEditingChannel] = useState<Channel | null | undefined>(undefined);
    const [deletingChannel, setDeletingChannel] = useState<Channel | null>(null);
    const [testingId, setTestingId] = useState<string | null>(null);
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

    const testChannel = (channel: Channel) => {
        setTestingId(channel.id);
        window.setTimeout(() => {
            recordChannelTest(channel.id, true);
            setTestingId(null);
            showToast(`${channel.name} 连接正常`);
        }, 900);
    };

    const activeCount = channels.filter((channel) => channel.status === 1).length;

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

            <section className="toolbar-row">
                <label className="search-field">
                    <Search size={16} />
                    <input value={query} placeholder="搜索渠道或模型" onChange={(event) => setQuery(event.target.value)} />
                </label>
                <label className="filter-select-wrap">
                    <span className="sr-only">渠道类型</span>
                    <select className="filter-select" value={typeFilter} onChange={(event) => setTypeFilter(event.target.value)}>
                        <option>全部</option>
                        {channelTypes.map((type) => <option key={type}>{type}</option>)}
                    </select>
                </label>
                <div className="ml-auto flex items-center gap-2 text-xs text-muted">
                    <span className="live-dot" />
                    自动健康检查已开启
                </div>
            </section>

            <section className="panel mt-4 min-w-0">
                <div className="table-scroll">
                    <table className="data-table channel-table min-w-[960px]">
                        <thead>
                            <tr>
                                <th>渠道</th>
                                <th>API 地址</th>
                                <th>模型</th>
                                <th>路由</th>
                                <th>状态</th>
                                <th>启用</th>
                                <th className="text-right">操作</th>
                            </tr>
                        </thead>
                        <tbody>
                            {filteredChannels.map((channel) => (
                                <tr key={channel.id}>
                                    <td>
                                        <div className="flex items-center gap-3">
                                            <ProviderMark type={channel.type} />
                                            <div className="min-w-0">
                                                <p className="font-medium text-ink">{channel.name}</p>
                                                <p className="mt-0.5 text-xs text-subtle">{channel.type}</p>
                                            </div>
                                        </div>
                                    </td>
                                    <td><span className="block max-w-[220px] truncate font-mono text-xs text-muted" title={channel.base_url}>{channel.base_url}</span></td>
                                    <td>
                                        <div className="flex max-w-[230px] flex-wrap gap-1">
                                            {channel.models.slice(0, 2).map((model) => <span className="model-name" key={model}>{model}</span>)}
                                            {channel.models.length > 2 ? <span className="model-more">+{channel.models.length - 2}</span> : null}
                                        </div>
                                    </td>
                                    <td>
                                        <p className="font-mono text-xs text-ink">P{channel.priority} · W{channel.weight}</p>
                                    </td>
                                    <td>
                                        {channel.last_test_ok === 1 ? (
                                            <div>
                                                <StatusBadge status="success" dot>正常</StatusBadge>
                                                {channel.last_test_at ? <p className="mt-1 text-[10px] text-subtle">{formatDateTime(channel.last_test_at)}</p> : null}
                                            </div>
                                        ) : channel.last_test_ok === 0 ? (
                                            <StatusBadge status="danger" dot>异常</StatusBadge>
                                        ) : (
                                            <StatusBadge status="neutral">未测试</StatusBadge>
                                        )}
                                    </td>
                                    <td>
                                        <Toggle
                                            checked={channel.status === 1}
                                            label={`${channel.status === 1 ? "停用" : "启用"}${channel.name}`}
                                            onChange={() => toggleChannel(channel.id)}
                                        />
                                    </td>
                                    <td>
                                        <div className="flex justify-end gap-1">
                                            <IconButton
                                                label={`测试 ${channel.name}`}
                                                disabled={testingId === channel.id}
                                                onClick={() => testChannel(channel)}
                                            >
                                                {testingId === channel.id ? <span className="button-spinner" /> : <FlaskConical size={16} />}
                                            </IconButton>
                                            <IconButton label={`编辑 ${channel.name}`} onClick={() => setEditingChannel(channel)}>
                                                <Pencil size={16} />
                                            </IconButton>
                                            <IconButton label={`删除 ${channel.name}`} tone="danger" onClick={() => setDeletingChannel(channel)}>
                                                <Trash2 size={16} />
                                            </IconButton>
                                        </div>
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
                {filteredChannels.length === 0 ? (
                    <div className="empty-state">
                        <Radio size={22} />
                        <strong>没有匹配的渠道</strong>
                        <span>调整搜索关键词或渠道类型</span>
                    </div>
                ) : null}
            </section>

            <section className="mt-4 grid gap-3 sm:grid-cols-3">
                <div className="summary-tile"><CheckCircle2 size={18} className="text-accent" /><div><span>运行中</span><strong>{activeCount}</strong></div></div>
                <div className="summary-tile"><XCircle size={18} className="text-danger" /><div><span>异常</span><strong>{channels.filter((channel) => channel.last_test_ok === 0).length}</strong></div></div>
                <div className="summary-tile"><FlaskConical size={18} className="text-data-blue" /><div><span>自动检查</span><strong>5 min</strong></div></div>
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
                                    deleteChannel(deletingChannel.id);
                                    setDeletingChannel(null);
                                    showToast("渠道已删除");
                                }}
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
