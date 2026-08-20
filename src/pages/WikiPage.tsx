import { useCallback, useEffect, useMemo, useState } from "react";
import {
    BookOpen,
    Check,
    ChevronRight,
    FileCode2,
    FileText,
    GitBranch,
    Globe2,
    Loader2,
    Network,
    Plus,
    RefreshCw,
    Search,
    Save,
    Settings2,
    Tag,
    Trash2,
    Upload,
    X,
} from "lucide-react";
import {
    wikiApi,
    type WikiGraphData,
    type WikiPage as WikiDocumentPage,
    type WikiProject,
    type WikiSearchResult,
    type WikiSource,
    type WikiTag,
} from "../lib/api";
import { ServiceSwitcher } from "../components/ServiceSwitcher";
import { IconButton, Modal, PageTitle, StatusBadge, Toast, Toggle } from "../components/ui";

type WikiTab = "overview" | "pages" | "sources" | "search" | "graph" | "settings";

function errorMessage(error: unknown): string {
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes("__TAURI_INTERNALS__") || message.includes("reading 'invoke'")) {
        return "Wiki 服务需要在 CrowAPI 桌面应用中运行";
    }
    return message || "Wiki 操作失败，请稍后重试";
}

function shortDate(value: string | null | undefined): string {
    if (!value) return "从未同步";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString("zh-CN", { dateStyle: "short", timeStyle: "short" });
}

function statNumber(stats: Record<string, unknown>, keys: string[]): number {
    for (const key of keys) {
        const value = stats[key];
        if (typeof value === "number") return value;
    }
    return 0;
}

export function WikiPage() {
    const [projects, setProjects] = useState<WikiProject[]>([]);
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [tab, setTab] = useState<WikiTab>("overview");
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [toast, setToast] = useState<string | null>(null);
    const [createOpen, setCreateOpen] = useState(false);
    const [deleteTarget, setDeleteTarget] = useState<WikiProject | null>(null);

    const selectedProject = useMemo(
        () => projects.find((project) => project.id === selectedId) ?? null,
        [projects, selectedId],
    );

    const loadProjects = useCallback(async () => {
        setLoading(true);
        try {
            const next = await wikiApi.getProjects();
            setProjects(next);
            setSelectedId((current) => current && next.some((project) => project.id === current) ? current : next[0]?.id ?? null);
            setError(null);
        } catch (err) {
            setError(errorMessage(err));
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        void loadProjects();
    }, [loadProjects]);

    const updateProject = async (id: string, input: Parameters<typeof wikiApi.updateProject>[1]) => {
        try {
            const updated = await wikiApi.updateProject(id, input);
            setProjects((current) => current.map((project) => project.id === id ? updated : project));
        } catch (err) {
            setError(errorMessage(err));
        }
    };

    const deleteProject = async () => {
        if (!deleteTarget) return;
        try {
            await wikiApi.deleteProject(deleteTarget.id);
            setDeleteTarget(null);
            setToast("Wiki 项目已删除");
            await loadProjects();
        } catch (err) {
            setError(errorMessage(err));
        }
    };

    const tabs: Array<{ key: WikiTab; label: string; icon: typeof BookOpen }> = [
        { key: "overview", label: "概览", icon: BookOpen },
        { key: "pages", label: "页面", icon: FileText },
        { key: "sources", label: "来源", icon: GitBranch },
        { key: "search", label: "搜索", icon: Search },
        { key: "graph", label: "图谱", icon: Network },
        { key: "settings", label: "设置", icon: Settings2 },
    ];

    return (
        <div className="page-enter wiki-page space-y-5">
            <PageTitle
                title="Wiki"
                meta="结构化页面、来源摄入与 wikilinks 关系图谱"
                action={<ServiceSwitcher />}
            />

            {error ? (
                <div className="flex items-start justify-between gap-3 rounded-lg border border-danger/25 bg-danger-soft px-4 py-3 text-sm text-danger" role="alert">
                    <span>{error}</span>
                    <IconButton label="关闭错误" onClick={() => setError(null)}><X size={15} /></IconButton>
                </div>
            ) : null}

            {loading ? (
                <div className="surface flex min-h-48 items-center justify-center rounded-lg text-muted">
                    <Loader2 className="mr-2 animate-spin" size={18} /> 正在读取 Wiki 项目
                </div>
            ) : projects.length === 0 ? (
                <div className="surface empty-state rounded-lg px-6 py-16 text-center">
                    <Network className="mx-auto mb-3 text-accent" size={28} />
                    <h2 className="text-base font-semibold text-ink">还没有 Wiki 项目</h2>
                    <p className="mx-auto mt-1 max-w-md text-sm text-muted">创建一个项目后，可以导入 Markdown 或代码来源，并在页面之间维护 wikilinks。</p>
                    <button type="button" className="action-primary mt-5" onClick={() => setCreateOpen(true)}>
                        <Plus size={15} /> 创建第一个项目
                    </button>
                </div>
            ) : selectedProject ? (
                <div className="grid min-w-0 gap-4 xl:grid-cols-[230px_minmax(0,1fr)]">
                    <aside className="surface min-w-0 rounded-lg p-3">
                        <div className="mb-2 flex items-center justify-between px-2">
                            <span className="text-[11px] font-semibold uppercase tracking-[0.12em] text-muted">项目</span>
                            <div className="flex items-center gap-1">
                                <IconButton label="新建 Wiki 项目" onClick={() => setCreateOpen(true)}>
                                    <Plus size={15} />
                                </IconButton>
                                <IconButton label="刷新项目" onClick={() => void loadProjects()}>
                                    <RefreshCw size={15} />
                                </IconButton>
                            </div>
                        </div>
                        <div className="space-y-1">
                            {projects.map((project) => (
                                <button
                                    key={project.id}
                                    type="button"
                                    className={`flex w-full items-center gap-2 rounded-md px-3 py-2.5 text-left text-sm transition-colors ${project.id === selectedProject.id ? "bg-accent-soft text-accent-ink" : "text-muted hover:bg-accent-soft/60 hover:text-ink"}`}
                                    aria-current={project.id === selectedProject.id ? "page" : undefined}
                                    onClick={() => { setSelectedId(project.id); setTab("overview"); }}
                                >
                                    <Network size={15} className="shrink-0" />
                                    <span className="min-w-0 flex-1 truncate">{project.name}</span>
                                    <ChevronRight size={14} className="shrink-0 opacity-60" />
                                </button>
                            ))}
                        </div>
                    </aside>

                    <section className="min-w-0 space-y-4">
                        <div className="surface rounded-lg p-2">
                            <div className="flex min-w-0 gap-1 overflow-x-auto" role="tablist" aria-label="Wiki 项目视图">
                                {tabs.map(({ key, label, icon: Icon }) => (
                                    <button
                                        key={key}
                                        type="button"
                                        role="tab"
                                        aria-selected={tab === key}
                                        className={`flex shrink-0 items-center gap-1.5 rounded-md px-3 py-2 text-sm font-medium transition-colors ${tab === key ? "bg-accent-soft text-accent-ink" : "text-muted hover:bg-accent-soft/60 hover:text-ink"}`}
                                        onClick={() => setTab(key)}
                                    >
                                        <Icon size={15} /> {label}
                                    </button>
                                ))}
                            </div>
                        </div>

                        {tab === "overview" ? <WikiOverview project={selectedProject} /> : null}
                        {tab === "pages" ? <WikiPages project={selectedProject} onError={setError} onSuccess={setToast} /> : null}
                        {tab === "sources" ? <WikiSources project={selectedProject} onError={setError} onSuccess={setToast} onProjectRefresh={loadProjects} /> : null}
                        {tab === "search" ? <WikiSearch project={selectedProject} onError={setError} /> : null}
                        {tab === "graph" ? <WikiGraph project={selectedProject} onError={setError} /> : null}
                        {tab === "settings" ? <WikiSettings project={selectedProject} onSave={(input) => updateProject(selectedProject.id, input)} onDelete={() => setDeleteTarget(selectedProject)} /> : null}

                        <div className="surface flex flex-wrap items-center justify-between gap-3 rounded-lg px-4 py-3 text-xs text-muted">
                            <span>目录：<code className="font-mono text-ink">{selectedProject.wiki_dir}</code></span>
                            <div className="flex items-center gap-3">
                                <span>最后摄入：{shortDate(selectedProject.last_ingest_at)}</span>
                                <Toggle
                                    checked={selectedProject.mcp_enabled === 1}
                                    label="Wiki MCP 暴露"
                                    onChange={(checked) => void updateProject(selectedProject.id, { mcp_enabled: checked ? 1 : 0 })}
                                />
                            </div>
                        </div>
                    </section>
                </div>
            ) : null}

            {createOpen ? <CreateWikiModal onClose={() => setCreateOpen(false)} onCreated={async (project) => { setCreateOpen(false); setToast("Wiki 项目已创建"); await loadProjects(); setSelectedId(project.id); }} /> : null}
            {deleteTarget ? (
                <Modal
                    title="删除 Wiki 项目"
                    description="项目目录和页面记录会一并删除，此操作无法撤销。"
                    onClose={() => setDeleteTarget(null)}
                    footer={(
                        <>
                            <button type="button" className="action-secondary" onClick={() => setDeleteTarget(null)}>取消</button>
                            <button type="button" className="action-danger" onClick={() => void deleteProject()}>删除项目</button>
                        </>
                    )}
                >
                    <p className="text-sm text-muted">即将删除 <strong className="text-ink">{deleteTarget.name}</strong>。</p>
                </Modal>
            ) : null}
            {toast ? <Toast message={toast} /> : null}
        </div>
    );
}

function WikiOverview({ project }: { project: WikiProject }) {
    const [stats, setStats] = useState<Record<string, unknown>>({});
    const [tags, setTags] = useState<WikiTag[]>([]);
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        let active = true;
        setLoading(true);
        Promise.all([wikiApi.getStats(project.id), wikiApi.getTags(project.id, 12)])
            .then(([nextStats, nextTags]) => { if (active) { setStats(nextStats); setTags(nextTags); } })
            .catch(() => { if (active) { setStats({}); setTags([]); } })
            .finally(() => { if (active) setLoading(false); });
        return () => { active = false; };
    }, [project.id]);

    const metrics = [
        ["页面", statNumber(stats, ["page_count", "pages", "total_pages"]) || project.page_count, FileText],
        ["来源", statNumber(stats, ["source_count", "sources", "total_sources"]) || project.source_count, GitBranch],
        ["活跃状态", project.status === 1 ? "运行中" : "已暂停", Network],
        ["MCP", project.mcp_enabled === 1 ? "已暴露" : "未暴露", Globe2],
    ] as const;

    return (
        <div className="space-y-4">
            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                {metrics.map(([label, value, Icon]) => (
                    <div key={label} className="surface rounded-lg p-4">
                        <div className="flex items-center justify-between text-muted"><span className="text-xs">{label}</span><Icon size={16} /></div>
                        <div className="mt-3 text-xl font-semibold text-ink">{loading ? "—" : value}</div>
                    </div>
                ))}
            </div>
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_280px]">
                <div className="surface rounded-lg p-5">
                    <div className="mb-3 flex items-center justify-between"><h2 className="text-sm font-semibold text-ink">项目说明</h2><StatusBadge status={project.status === 1 ? "success" : "neutral"} dot>{project.status === 1 ? "可用" : "暂停"}</StatusBadge></div>
                    <p className="whitespace-pre-wrap text-sm leading-6 text-muted">{project.description || "还没有项目说明。可以在设置中补充 Wiki 的维护边界和写作约定。"}</p>
                    <div className="mt-5 grid gap-3 text-xs text-muted sm:grid-cols-2">
                        <div><span className="text-muted">摄入模型</span><div className="mt-1 font-mono text-ink">{project.ingest_model || "自动选择"}</div></div>
                        <div><span className="text-muted">对话模型</span><div className="mt-1 font-mono text-ink">{project.chat_model || "自动选择"}</div></div>
                    </div>
                </div>
                <div className="surface rounded-lg p-5">
                    <div className="mb-3 flex items-center gap-2 text-sm font-semibold text-ink"><Tag size={15} /> 热门标签</div>
                    {tags.length ? <div className="flex flex-wrap gap-2">{tags.map((tag) => <span key={tag.word} className="rounded-md bg-accent-soft px-2 py-1 text-xs text-accent-ink">{tag.word} <span className="opacity-70">{tag.count}</span></span>)}</div> : <p className="text-sm text-muted">摄入页面后会在这里显示标签。</p>}
                </div>
            </div>
        </div>
    );
}

function WikiPages({ project, onError, onSuccess }: { project: WikiProject; onError: (message: string) => void; onSuccess: (message: string) => void }) {
    const [pages, setPages] = useState<WikiDocumentPage[]>([]);
    const [selectedPath, setSelectedPath] = useState<string | null>(null);
    const [content, setContent] = useState("");
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);

    const loadPages = useCallback(async () => {
        setLoading(true);
        try {
            const next = await wikiApi.getPages(project.id);
            setPages(next);
            setSelectedPath((current) => current && next.some((page) => page.path === current) ? current : next[0]?.path ?? null);
        } catch (err) { onError(errorMessage(err)); } finally { setLoading(false); }
    }, [onError, project.id]);

    useEffect(() => { void loadPages(); }, [loadPages]);
    useEffect(() => {
        if (!selectedPath) { setContent(""); return; }
        let active = true;
        wikiApi.getPage(project.id, selectedPath).then((page) => { if (active) setContent(page.content ?? ""); }).catch((err) => { if (active) onError(errorMessage(err)); });
        return () => { active = false; };
    }, [onError, project.id, selectedPath]);

    const selected = pages.find((page) => page.path === selectedPath);
    const save = async () => {
        if (!selectedPath) return;
        setSaving(true);
        try { await wikiApi.savePage(project.id, selectedPath, content); onSuccess("Wiki 页面已保存"); await loadPages(); }
        catch (err) { onError(errorMessage(err)); }
        finally { setSaving(false); }
    };

    return (
        <div className="grid min-w-0 gap-4 lg:grid-cols-[260px_minmax(0,1fr)]">
            <div className="surface min-w-0 rounded-lg p-3">
                <div className="mb-2 flex items-center justify-between px-2"><span className="text-xs font-semibold text-ink">页面目录</span><button type="button" className="icon-button" title="刷新页面" aria-label="刷新页面" onClick={() => void loadPages()}><RefreshCw size={14} /></button></div>
                {loading ? <div className="px-2 py-8 text-center text-xs text-muted"><Loader2 className="mx-auto mb-2 animate-spin" size={16} />读取中</div> : pages.length ? <div className="max-h-[520px] space-y-1 overflow-auto">{pages.map((page) => <button key={page.path} type="button" className={`flex w-full items-start gap-2 rounded-md px-3 py-2 text-left text-xs ${page.path === selectedPath ? "bg-accent-soft text-accent-ink" : "text-muted hover:bg-accent-soft/60 hover:text-ink"}`} onClick={() => setSelectedPath(page.path)}><FileCode2 size={14} className="mt-0.5 shrink-0" /><span className="min-w-0"><span className="block truncate font-medium">{page.title}</span><span className="mt-0.5 block truncate font-mono opacity-70">{page.path}</span></span></button>)}</div> : <p className="px-2 py-8 text-center text-xs text-muted">暂无页面，请先摄入来源。</p>}
            </div>
            <div className="surface min-w-0 rounded-lg p-4">
                {selected ? <>
                    <div className="mb-3 flex flex-wrap items-center justify-between gap-3"><div><h2 className="text-sm font-semibold text-ink">{selected.title}</h2><p className="mt-1 font-mono text-[11px] text-muted">{selected.path}</p></div><button type="button" className="action-primary" disabled={saving} onClick={() => void save()}>{saving ? <Loader2 className="animate-spin" size={15} /> : <Save size={15} />} 保存页面</button></div>
                    <textarea value={content} onChange={(event) => setContent(event.target.value)} className="min-h-[420px] w-full resize-y rounded-md border border-line bg-canvas px-4 py-3 font-mono text-sm leading-6 text-ink outline-none focus:border-accent" aria-label="Wiki 页面内容" />
                    <div className="mt-3 flex flex-wrap gap-3 text-xs text-muted"><span>{selected.token_count} tokens</span><span>类型：{selected.page_type}</span><span>更新：{shortDate(selected.updated_at)}</span></div>
                </> : <div className="flex min-h-[460px] items-center justify-center text-sm text-muted">选择一个页面开始编辑。</div>}
            </div>
        </div>
    );
}

function WikiSources({ project, onError, onSuccess, onProjectRefresh }: { project: WikiProject; onError: (message: string) => void; onSuccess: (message: string) => void; onProjectRefresh: () => Promise<void> }) {
    const [sources, setSources] = useState<WikiSource[]>([]);
    const [loading, setLoading] = useState(true);
    const [adding, setAdding] = useState(false);
    const [ingestingId, setIngestingId] = useState<string | null>(null);
    const [deleteTarget, setDeleteTarget] = useState<WikiSource | null>(null);
    const [sourceType, setSourceType] = useState("markdown");
    const [filename, setFilename] = useState("notes.md");
    const [sourceUrl, setSourceUrl] = useState("");
    const [content, setContent] = useState("");

    const load = useCallback(async () => {
        setLoading(true);
        try { setSources(await wikiApi.getSources(project.id)); } catch (err) { onError(errorMessage(err)); } finally { setLoading(false); }
    }, [onError, project.id]);
    useEffect(() => { void load(); }, [load]);

    const add = async () => {
        if (!filename.trim()) return;
        setAdding(true);
        try {
            await wikiApi.addSource(project.id, { source_type: sourceType, filename: filename.trim(), source_url: sourceUrl.trim() || undefined, content: content || undefined });
            setContent(""); setSourceUrl(""); setFilename("notes.md");
            onSuccess("来源已加入队列"); await load(); await onProjectRefresh();
        } catch (err) { onError(errorMessage(err)); }
        finally { setAdding(false); }
    };
    const ingest = async (source: WikiSource) => {
        setIngestingId(source.id);
        try { await wikiApi.ingestSource(project.id, source.id); onSuccess(`${source.filename} 已完成摄入`); await load(); await onProjectRefresh(); }
        catch (err) { onError(errorMessage(err)); }
        finally { setIngestingId(null); }
    };
    const remove = async () => {
        if (!deleteTarget) return;
        try { await wikiApi.deleteSource(deleteTarget.id); setDeleteTarget(null); onSuccess("来源已删除"); await load(); await onProjectRefresh(); }
        catch (err) { onError(errorMessage(err)); }
    };

    return (
        <div className="space-y-4">
            <div className="surface rounded-lg p-4">
                <div className="mb-3 flex flex-wrap items-center justify-between gap-3"><div><h2 className="text-sm font-semibold text-ink">来源摄入</h2><p className="mt-1 text-xs text-muted">支持在桌面端写入 Markdown 内容，也可以记录 URL 或本地路径。</p></div><button type="button" className="action-primary" onClick={() => void add()} disabled={adding || !filename.trim()}>{adding ? <Loader2 className="animate-spin" size={15} /> : <Upload size={15} />} 添加来源</button></div>
                <div className="grid gap-3 md:grid-cols-[150px_minmax(0,1fr)]">
                    <label className="text-xs text-muted">类型<select value={sourceType} onChange={(event) => setSourceType(event.target.value)} className="field mt-1 w-full"><option value="markdown">Markdown</option><option value="code">代码</option><option value="url">URL</option><option value="file">本地文件</option></select></label>
                    <label className="text-xs text-muted">文件名或路径<input value={filename} onChange={(event) => setFilename(event.target.value)} className="field mt-1 w-full" placeholder="notes.md" /></label>
                </div>
                {sourceType === "url" ? <label className="mt-3 block text-xs text-muted">来源 URL<input value={sourceUrl} onChange={(event) => setSourceUrl(event.target.value)} className="field mt-1 w-full" placeholder="https://..." /></label> : null}
                <label className="mt-3 block text-xs text-muted">内容（可选）<textarea value={content} onChange={(event) => setContent(event.target.value)} className="field mt-1 min-h-24 w-full resize-y font-mono" placeholder="粘贴 Markdown 或代码内容" /></label>
            </div>
            <div className="surface overflow-hidden rounded-lg">
                <div className="flex items-center justify-between border-b border-line px-4 py-3"><h2 className="text-sm font-semibold text-ink">已登记来源</h2><button type="button" className="action-secondary" onClick={() => void wikiApi.rescanSources(project.id).then(() => { onSuccess("待处理来源已重新扫描"); return load(); }).catch((err) => onError(errorMessage(err)))}><RefreshCw size={14} /> 重新扫描</button></div>
                {loading ? <div className="p-8 text-center text-sm text-muted"><Loader2 className="mx-auto mb-2 animate-spin" size={17} />读取中</div> : sources.length ? <div className="divide-y divide-line">{sources.map((source) => <div key={source.id} className="flex flex-wrap items-center gap-3 px-4 py-3"><div className="flex min-w-0 flex-1 items-center gap-3"><GitBranch className="shrink-0 text-accent" size={17} /><div className="min-w-0"><div className="truncate text-sm font-medium text-ink">{source.filename}</div><div className="mt-1 flex flex-wrap gap-2 text-[11px] text-muted"><span>{source.source_type}</span><span>{source.page_count} 页面</span><span>{shortDate(source.ingested_at)}</span></div></div></div><StatusBadge status={source.status === "ingested" ? "success" : source.status === "failed" ? "danger" : "warning"}>{source.status}</StatusBadge><div className="flex items-center gap-1"><button type="button" className="action-secondary" disabled={ingestingId === source.id} onClick={() => void ingest(source)}>{ingestingId === source.id ? <Loader2 className="animate-spin" size={14} /> : <RefreshCw size={14} />} 摄入</button><IconButton label={`删除来源 ${source.filename}`} tone="danger" onClick={() => setDeleteTarget(source)}><Trash2 size={15} /></IconButton></div>{source.error_message ? <p className="basis-full text-xs text-danger">{source.error_message}</p> : null}</div>)}</div> : <div className="p-10 text-center text-sm text-muted">还没有来源。</div>}
            </div>
            {deleteTarget ? <Modal title="删除来源" description="删除来源记录不会影响其他 Wiki 项目。" onClose={() => setDeleteTarget(null)} footer={<><button type="button" className="action-secondary" onClick={() => setDeleteTarget(null)}>取消</button><button type="button" className="action-danger" onClick={() => void remove()}>删除来源</button></>}><p className="text-sm text-muted">确定删除 <strong className="text-ink">{deleteTarget.filename}</strong>？</p></Modal> : null}
        </div>
    );
}

function WikiSearch({ project, onError }: { project: WikiProject; onError: (message: string) => void }) {
    const [query, setQuery] = useState("");
    const [results, setResults] = useState<WikiSearchResult[]>([]);
    const [loading, setLoading] = useState(false);
    const search = async () => {
        if (!query.trim()) return;
        setLoading(true);
        try { setResults(await wikiApi.search(project.id, query.trim(), 20)); } catch (err) { onError(errorMessage(err)); } finally { setLoading(false); }
    };
    return <div className="surface rounded-lg p-4"><div className="flex gap-2"><input value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); void search(); } }} className="field min-w-0 flex-1" placeholder="搜索标题、路径或页面内容" aria-label="搜索 Wiki" /><button type="button" className="action-primary" onClick={() => void search()} disabled={loading || !query.trim()}>{loading ? <Loader2 className="animate-spin" size={15} /> : <Search size={15} />} 搜索</button></div><div className="mt-4 divide-y divide-line">{results.length ? results.map((result) => <article key={result.page_id} className="py-3"><div className="flex items-center justify-between gap-3"><h3 className="text-sm font-medium text-ink">{result.title}</h3><span className="font-mono text-[11px] text-accent">{result.score.toFixed(2)}</span></div><p className="mt-1 font-mono text-[11px] text-muted">{result.path}</p>{result.snippet ? <p className="mt-2 text-sm leading-6 text-muted">{result.snippet}</p> : null}</article>) : <p className="py-12 text-center text-sm text-muted">输入关键词开始搜索。</p>}</div></div>;
}

function WikiGraph({ project, onError }: { project: WikiProject; onError: (message: string) => void }) {
    const [graph, setGraph] = useState<WikiGraphData | null>(null);
    const [loading, setLoading] = useState(true);
    useEffect(() => { let active = true; setLoading(true); wikiApi.getGraph(project.id).then((next) => { if (active) setGraph(next); }).catch((err) => { if (active) onError(errorMessage(err)); }).finally(() => { if (active) setLoading(false); }); return () => { active = false; }; }, [onError, project.id]);
    if (loading) return <div className="surface flex min-h-48 items-center justify-center rounded-lg text-sm text-muted"><Loader2 className="mr-2 animate-spin" size={17} />正在构建关系图谱</div>;
    if (!graph || graph.nodes.length === 0) return <div className="surface rounded-lg p-12 text-center text-sm text-muted"><Network className="mx-auto mb-3 text-accent" size={24} />暂无 wikilinks 关系。页面之间建立 `[[页面路径]]` 链接后会显示在这里。</div>;
    return <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_300px]"><div className="surface rounded-lg p-4"><div className="mb-3 flex items-center justify-between"><h2 className="text-sm font-semibold text-ink">页面节点</h2><span className="text-xs text-muted">{graph.nodes.length} 节点 / {graph.edges.length} 连接</span></div><div className="grid gap-2 sm:grid-cols-2">{graph.nodes.map((node) => <div key={node.id} className="rounded-md border border-line bg-canvas px-3 py-2"><div className="flex items-center gap-2 text-sm font-medium text-ink"><Network size={14} className="text-accent" />{node.label}</div><div className="mt-1 flex justify-between text-[11px] text-muted"><span className="truncate font-mono">{node.path || node.node_type}</span><span>{node.link_count} links</span></div></div>)}</div></div><div className="surface rounded-lg p-4"><h2 className="mb-3 text-sm font-semibold text-ink">关系边</h2><div className="space-y-2">{graph.edges.map((edge, index) => <div key={`${edge.source}-${edge.target}-${index}`} className="text-xs text-muted"><span className="font-mono text-ink">{edge.source}</span><span className="mx-2 text-accent">→</span><span className="font-mono text-ink">{edge.target}</span><span className="ml-2 opacity-70">×{edge.weight}</span></div>)}</div></div></div>;
}

function WikiSettings({ project, onSave, onDelete }: { project: WikiProject; onSave: (input: Parameters<typeof wikiApi.updateProject>[1]) => Promise<void>; onDelete: () => void }) {
    const [name, setName] = useState(project.name);
    const [description, setDescription] = useState(project.description ?? "");
    const [ingestModel, setIngestModel] = useState(project.ingest_model ?? "");
    const [chatModel, setChatModel] = useState(project.chat_model ?? "");
    const [schema, setSchema] = useState(project.schema_text ?? "");
    const [saving, setSaving] = useState(false);
    useEffect(() => { setName(project.name); setDescription(project.description ?? ""); setIngestModel(project.ingest_model ?? ""); setChatModel(project.chat_model ?? ""); setSchema(project.schema_text ?? ""); }, [project]);
    const save = async () => { setSaving(true); try { await onSave({ name: name.trim(), description: description.trim(), ingest_model: ingestModel.trim() || undefined, chat_model: chatModel.trim() || undefined, schema_text: schema || undefined }); } finally { setSaving(false); } };
    return <div className="surface rounded-lg p-5"><div className="mb-5 flex flex-wrap items-start justify-between gap-3"><div><h2 className="text-sm font-semibold text-ink">项目设置</h2><p className="mt-1 text-xs text-muted">这些字段会影响 Wiki 摄入和 MCP 暴露。</p></div><div className="flex gap-2"><button type="button" className="action-danger" onClick={onDelete}><Trash2 size={14} /> 删除项目</button><button type="button" className="action-primary" disabled={saving || !name.trim()} onClick={() => void save()}>{saving ? <Loader2 className="animate-spin" size={15} /> : <Check size={15} />} 保存设置</button></div></div><div className="grid gap-4 md:grid-cols-2"><label className="text-xs text-muted">项目名称<input value={name} onChange={(event) => setName(event.target.value)} className="field mt-1 w-full" /></label><label className="text-xs text-muted">描述<input value={description} onChange={(event) => setDescription(event.target.value)} className="field mt-1 w-full" /></label><label className="text-xs text-muted">摄入模型<input value={ingestModel} onChange={(event) => setIngestModel(event.target.value)} className="field mt-1 w-full font-mono" placeholder="自动选择" /></label><label className="text-xs text-muted">对话模型<input value={chatModel} onChange={(event) => setChatModel(event.target.value)} className="field mt-1 w-full font-mono" placeholder="自动选择" /></label></div><label className="mt-4 block text-xs text-muted">Wiki Schema<textarea value={schema} onChange={(event) => setSchema(event.target.value)} className="field mt-1 min-h-52 w-full resize-y font-mono text-xs" placeholder="定义页面结构、标签和 wikilinks 规则" /></label></div>;
}

function CreateWikiModal({ onClose, onCreated }: { onClose: () => void; onCreated: (project: WikiProject) => Promise<void> }) {
    const [name, setName] = useState("");
    const [description, setDescription] = useState("");
    const [saving, setSaving] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const create = async () => { if (!name.trim()) return; setSaving(true); setError(null); try { const project = await wikiApi.createProject({ name: name.trim(), description: description.trim() || undefined }); await onCreated(project); } catch (err) { setError(errorMessage(err)); } finally { setSaving(false); } };
    return <Modal title="新建 Wiki 项目" description="为一组相关文档创建独立的页面目录和标签空间。" onClose={onClose} footer={<><button type="button" className="action-secondary" onClick={onClose}>取消</button><button type="button" className="action-primary" disabled={saving || !name.trim()} onClick={() => void create()}>{saving ? <Loader2 className="animate-spin" size={15} /> : <Plus size={15} />} 创建项目</button></>}><div className="space-y-4">{error ? <p className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger" role="alert">{error}</p> : null}<label className="block text-xs text-muted">项目名称<input autoFocus value={name} onChange={(event) => setName(event.target.value)} className="field mt-1 w-full" placeholder="例如：CrowAPI 架构" /></label><label className="block text-xs text-muted">描述<textarea value={description} onChange={(event) => setDescription(event.target.value)} className="field mt-1 min-h-24 w-full resize-y" placeholder="这个 Wiki 服务什么内容？" /></label></div></Modal>;
}
