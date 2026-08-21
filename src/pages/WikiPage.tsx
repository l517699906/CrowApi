import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
    type WikiProject,
    type WikiSource,
} from "../lib/api";
import { ServiceSwitcher } from "../components/ServiceSwitcher";
import { IconButton, Modal, PageTitle, StatusBadge, Toast, Toggle } from "../components/ui";
import { useBackgroundTasks } from "../hooks/useBackgroundTasks";
import { errorMessage, queryKeys } from "../lib/query";

type WikiTab = "overview" | "pages" | "sources" | "search" | "graph" | "settings";

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
    const queryClient = useQueryClient();
    const projectsQuery = useQuery({
        queryKey: queryKeys.wikiProjects,
        queryFn: wikiApi.getProjects,
    });
    const refetchProjects = projectsQuery.refetch;
    const projects = projectsQuery.data ?? [];
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [tab, setTab] = useState<WikiTab>("overview");
    const [error, setError] = useState<string | null>(null);
    const [toast, setToast] = useState<string | null>(null);
    const [createOpen, setCreateOpen] = useState(false);
    const [deleteTarget, setDeleteTarget] = useState<WikiProject | null>(null);
    const showToast = useCallback((message: string) => {
        setToast(message);
        window.setTimeout(() => setToast(null), 1_800);
    }, []);

    const selectedProject = useMemo(
        () => projects.find((project) => project.id === selectedId) ?? null,
        [projects, selectedId],
    );

    useEffect(() => {
        setSelectedId((current) => (
            current && projects.some((project) => project.id === current)
                ? current
                : projects[0]?.id ?? null
        ));
    }, [projects]);

    const refreshProjects = useCallback(async () => {
        await refetchProjects();
    }, [refetchProjects]);

    const updateMutation = useMutation({
        mutationFn: ({ id, input }: { id: string; input: Parameters<typeof wikiApi.updateProject>[1] }) => (
            wikiApi.updateProject(id, input)
        ),
        onSuccess: (updated) => {
            queryClient.setQueryData<WikiProject[]>(queryKeys.wikiProjects, (current) => (
                (current ?? []).map((project) => project.id === updated.id ? updated : project)
            ));
            queryClient.setQueryData(queryKeys.wikiProject(updated.id), updated);
            void Promise.all([
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiStats(updated.id) }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiTags(updated.id) }),
            ]);
        },
    });

    const updateProject = async (id: string, input: Parameters<typeof wikiApi.updateProject>[1]) => {
        try {
            await updateMutation.mutateAsync({ id, input });
            showToast("Wiki 项目已更新");
        } catch (err) {
            setError(errorMessage(err));
        }
    };

    const deleteMutation = useMutation({
        mutationFn: wikiApi.deleteProject,
        onSuccess: async (_, id) => {
            queryClient.setQueryData<WikiProject[]>(queryKeys.wikiProjects, (current) => (
                (current ?? []).filter((project) => project.id !== id)
            ));
            await queryClient.removeQueries({ queryKey: queryKeys.wikiProject(id) });
            setDeleteTarget(null);
            showToast("Wiki 项目已删除");
        },
    });

    const deleteProject = async () => {
        if (!deleteTarget) return;
        try {
            await deleteMutation.mutateAsync(deleteTarget.id);
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

            {projectsQuery.isPending ? (
                <div className="surface flex min-h-48 items-center justify-center rounded-lg text-muted">
                    <Loader2 className="mr-2 animate-spin" size={18} /> 正在读取 Wiki 项目
                </div>
            ) : projectsQuery.error ? (
                <div className="surface rounded-lg px-6 py-12 text-center">
                    <p className="text-sm text-danger">{errorMessage(projectsQuery.error)}</p>
                    <button type="button" className="action-secondary mt-4" onClick={() => void projectsQuery.refetch()}>
                        <RefreshCw size={14} /> 重试
                    </button>
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
                                <IconButton label="刷新项目" onClick={() => void refreshProjects()}>
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
                        {tab === "pages" ? <WikiPages project={selectedProject} onError={setError} onSuccess={showToast} /> : null}
                        {tab === "sources" ? <WikiSources project={selectedProject} onError={setError} onSuccess={showToast} onProjectRefresh={refreshProjects} /> : null}
                        {tab === "search" ? <WikiSearch project={selectedProject} /> : null}
                        {tab === "graph" ? <WikiGraph project={selectedProject} /> : null}
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

            {createOpen ? <CreateWikiModal onClose={() => setCreateOpen(false)} onCreated={async (project) => { setCreateOpen(false); showToast("Wiki 项目已创建"); queryClient.setQueryData<WikiProject[]>(queryKeys.wikiProjects, (current) => [...(current ?? []).filter((item) => item.id !== project.id), project]); setSelectedId(project.id); }} /> : null}
            {deleteTarget ? (
                <Modal
                    title="删除 Wiki 项目"
                    description="项目目录和页面记录会一并删除，此操作无法撤销。"
                    onClose={() => setDeleteTarget(null)}
                    footer={(
                        <>
                            <button type="button" className="action-secondary" onClick={() => setDeleteTarget(null)}>取消</button>
                            <button type="button" className="action-danger" disabled={deleteMutation.isPending} onClick={() => void deleteProject()}>{deleteMutation.isPending ? "删除中..." : "删除项目"}</button>
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
    const statsQuery = useQuery({
        queryKey: queryKeys.wikiStats(project.id),
        queryFn: () => wikiApi.getStats(project.id),
    });
    const tagsQuery = useQuery({
        queryKey: queryKeys.wikiTags(project.id),
        queryFn: () => wikiApi.getTags(project.id, 12),
    });
    const stats = statsQuery.data ?? {};
    const tags = tagsQuery.data ?? [];
    const loading = statsQuery.isPending || tagsQuery.isPending;
    const statsError = statsQuery.error ?? tagsQuery.error;

    const metrics = [
        ["页面", statsQuery.data ? statNumber(stats, ["page_count", "pages", "total_pages"]) : project.page_count, FileText],
        ["来源", statsQuery.data ? statNumber(stats, ["source_count", "sources", "total_sources"]) : project.source_count, GitBranch],
        ["活跃状态", project.status === 1 ? "运行中" : "已暂停", Network],
        ["MCP", project.mcp_enabled === 1 ? "已暴露" : "未暴露", Globe2],
    ] as const;

    return (
        <div className="space-y-4">
            {statsError ? (
                <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-danger/25 bg-danger-soft px-3 py-2 text-xs text-danger" role="alert">
                    <span>{errorMessage(statsError)}</span>
                    <button type="button" className="action-secondary" onClick={() => { void statsQuery.refetch(); void tagsQuery.refetch(); }}>
                        <RefreshCw size={13} /> 重试
                    </button>
                </div>
            ) : null}
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
    const queryClient = useQueryClient();
    const pagesQuery = useQuery({
        queryKey: queryKeys.wikiPages(project.id),
        queryFn: () => wikiApi.getPages(project.id),
    });
    const pages = pagesQuery.data ?? [];
    const [selectedPath, setSelectedPath] = useState<string | null>(null);
    const [content, setContent] = useState("");
    useEffect(() => {
        setSelectedPath((current) => (
            current && pages.some((page) => page.path === current)
                ? current
                : pages[0]?.path ?? null
        ));
    }, [pages]);

    const pageQuery = useQuery({
        queryKey: queryKeys.wikiPage(project.id, selectedPath ?? ""),
        queryFn: () => wikiApi.getPage(project.id, selectedPath as string),
        enabled: Boolean(selectedPath),
    });
    useEffect(() => {
        setContent(selectedPath ? pageQuery.data?.content ?? "" : "");
    }, [pageQuery.data?.content, selectedPath]);

    const selected = pages.find((page) => page.path === selectedPath);
    const saveMutation = useMutation({
        mutationFn: () => wikiApi.savePage(project.id, selectedPath as string, content),
        onSuccess: async () => {
            await Promise.all([
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiPages(project.id) }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiPage(project.id, selectedPath ?? "") }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiStats(project.id) }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiTags(project.id) }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiGraph(project.id) }),
            ]);
            onSuccess("Wiki 页面已保存");
        },
        onError: (err) => onError(errorMessage(err)),
    });

    return (
        <div className="grid min-w-0 gap-4 lg:grid-cols-[260px_minmax(0,1fr)]">
            <div className="surface min-w-0 rounded-lg p-3">
                <div className="mb-2 flex items-center justify-between px-2"><span className="text-xs font-semibold text-ink">页面目录</span><button type="button" className="icon-button" title="刷新页面" aria-label="刷新页面" onClick={() => void pagesQuery.refetch()}><RefreshCw size={14} /></button></div>
                {pagesQuery.isPending ? <div className="px-2 py-8 text-center text-xs text-muted"><Loader2 className="mx-auto mb-2 animate-spin" size={16} />读取中</div> : pagesQuery.error ? <div className="px-2 py-8 text-center text-xs text-danger"><p>{errorMessage(pagesQuery.error)}</p><button type="button" className="action-secondary mt-3" onClick={() => void pagesQuery.refetch()}><RefreshCw size={13} /> 重试</button></div> : pages.length ? <div className="max-h-[520px] space-y-1 overflow-auto">{pages.map((page) => <button key={page.path} type="button" className={`flex w-full items-start gap-2 rounded-md px-3 py-2 text-left text-xs ${page.path === selectedPath ? "bg-accent-soft text-accent-ink" : "text-muted hover:bg-accent-soft/60 hover:text-ink"}`} onClick={() => setSelectedPath(page.path)}><FileCode2 size={14} className="mt-0.5 shrink-0" /><span className="min-w-0"><span className="block truncate font-medium">{page.title}</span><span className="mt-0.5 block truncate font-mono opacity-70">{page.path}</span></span></button>)}</div> : <p className="px-2 py-8 text-center text-xs text-muted">暂无页面，请先摄入来源。</p>}
            </div>
            <div className="surface min-w-0 rounded-lg p-4">
                {selected ? <>
                    <div className="mb-3 flex flex-wrap items-center justify-between gap-3"><div><h2 className="text-sm font-semibold text-ink">{selected.title}</h2><p className="mt-1 font-mono text-[11px] text-muted">{selected.path}</p></div><button type="button" className="action-primary" disabled={saveMutation.isPending || pageQuery.isPending} onClick={() => void saveMutation.mutate()}>{saveMutation.isPending ? <Loader2 className="animate-spin" size={15} /> : <Save size={15} />} 保存页面</button></div>
                    {pageQuery.error ? <div className="mb-3 flex items-center justify-between gap-2 rounded-md bg-danger-soft px-3 py-2 text-xs text-danger" role="alert"><span>{errorMessage(pageQuery.error)}</span><button type="button" className="action-secondary" onClick={() => void pageQuery.refetch()}><RefreshCw size={13} /> 重试</button></div> : null}
                    <textarea value={content} onChange={(event) => setContent(event.target.value)} disabled={pageQuery.isPending} className="min-h-[420px] w-full resize-y rounded-md border border-line bg-canvas px-4 py-3 font-mono text-sm leading-6 text-ink outline-none focus:border-accent disabled:opacity-60" aria-label="Wiki 页面内容" />
                    <div className="mt-3 flex flex-wrap gap-3 text-xs text-muted"><span>{selected.token_count} tokens</span><span>类型：{selected.page_type}</span><span>更新：{shortDate(selected.updated_at)}</span></div>
                </> : <div className="flex min-h-[460px] items-center justify-center text-sm text-muted">选择一个页面开始编辑。</div>}
            </div>
        </div>
    );
}

function WikiSources({ project, onError, onSuccess, onProjectRefresh }: { project: WikiProject; onError: (message: string) => void; onSuccess: (message: string) => void; onProjectRefresh: () => Promise<void> }) {
    const queryClient = useQueryClient();
    const sourcesQuery = useQuery({
        queryKey: queryKeys.wikiSources(project.id),
        queryFn: () => wikiApi.getSources(project.id),
    });
    const refetchSources = sourcesQuery.refetch;
    const sources = sourcesQuery.data ?? [];
    const loading = sourcesQuery.isPending;
    const [adding, setAdding] = useState(false);
    const [ingestingId, setIngestingId] = useState<string | null>(null);
    const [deleteTarget, setDeleteTarget] = useState<WikiSource | null>(null);
    const [sourceType, setSourceType] = useState("markdown");
    const [filename, setFilename] = useState("notes.md");
    const [sourceUrl, setSourceUrl] = useState("");
    const [content, setContent] = useState("");
    const taskQuery = useBackgroundTasks({
        domain: "wiki",
        resourceType: "wiki_project",
        resourceId: project.id,
        limit: 30,
    });
    const previousTaskStatuses = useRef(new Map<string, string>());
    const activeSourceIds = useMemo(() => new Set(
        (taskQuery.data ?? [])
            .filter((task) => task.status === "pending" || task.status === "running")
            .map((task) => task.subjectId)
            .filter((sourceId): sourceId is string => Boolean(sourceId)),
    ), [taskQuery.data]);
    useEffect(() => {
        previousTaskStatuses.current.clear();
    }, [project.id]);

    useEffect(() => {
        let completed = false;
        for (const task of taskQuery.data ?? []) {
            const previous = previousTaskStatuses.current.get(task.id);
            previousTaskStatuses.current.set(task.id, task.status);
            if ((previous === "pending" || previous === "running")
                && task.status !== "pending"
                && task.status !== "running") {
                completed = true;
                if (task.status === "failed") {
                    onError(task.errorMessage || "Wiki 摄入失败");
                } else if (task.status === "succeeded") {
                    onSuccess("Wiki 摄入完成");
                }
            }
        }
        if (completed) {
            void Promise.all([
                refetchSources(),
                onProjectRefresh(),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiStats(project.id) }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiTags(project.id) }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiPages(project.id) }),
                queryClient.invalidateQueries({ queryKey: queryKeys.wikiGraph(project.id) }),
            ]);
        }
    }, [onError, onProjectRefresh, project.id, queryClient, refetchSources, onSuccess, taskQuery.data]);

    const invalidateSources = useCallback(() => (
        queryClient.invalidateQueries({ queryKey: queryKeys.wikiSources(project.id) })
    ), [project.id, queryClient]);

    const addMutation = useMutation({
        mutationFn: () => wikiApi.addSource(project.id, {
            source_type: sourceType,
            filename: filename.trim(),
            source_url: sourceUrl.trim() || undefined,
            content: content || undefined,
        }),
        onSuccess: async () => {
            setContent(""); setSourceUrl(""); setFilename("notes.md");
            await Promise.all([invalidateSources(), onProjectRefresh()]);
            onSuccess("来源已加入队列");
        },
        onError: (err) => onError(errorMessage(err)),
    });

    const ingestMutation = useMutation({
        mutationFn: (source: WikiSource) => wikiApi.ingestSource(project.id, source.id),
        onSuccess: async (result, source) => {
            await invalidateSources();
            onSuccess(`${source.filename} 已开始摄入，任务 ${result.task_id.slice(0, 8)}`);
        },
        onError: (err) => onError(errorMessage(err)),
    });

    const deleteMutation = useMutation({
        mutationFn: (source: WikiSource) => wikiApi.deleteSource(source.id),
        onSuccess: async (_, source) => {
            setDeleteTarget(null);
            await Promise.all([invalidateSources(), onProjectRefresh()]);
            onSuccess(`${source.filename} 已删除`);
        },
        onError: (err) => onError(errorMessage(err)),
    });

    const rescanMutation = useMutation({
        mutationFn: () => wikiApi.rescanSources(project.id),
        onSuccess: async () => {
            await invalidateSources();
            onSuccess("待处理来源已重新扫描");
        },
        onError: (err) => onError(errorMessage(err)),
    });

    const add = async () => {
        if (!filename.trim() || adding) return;
        setAdding(true);
        try { await addMutation.mutateAsync(); } catch { /* Mutation callback reports the error. */ } finally { setAdding(false); }
    };
    const ingest = async (source: WikiSource) => {
        setIngestingId(source.id);
        try { await ingestMutation.mutateAsync(source); } catch { /* Mutation callback reports the error. */ } finally { setIngestingId(null); }
    };
    const remove = async () => {
        if (!deleteTarget) return;
        try { await deleteMutation.mutateAsync(deleteTarget); } catch { /* Mutation callback reports the error. */ }
    };

    return (
        <div className="space-y-4">
            <div className="surface rounded-lg p-4">
                <div className="mb-3 flex flex-wrap items-center justify-between gap-3"><div><h2 className="text-sm font-semibold text-ink">来源摄入</h2><p className="mt-1 text-xs text-muted">支持在桌面端写入 Markdown 内容，也可以记录 URL 或本地路径。</p></div><button type="button" className="action-primary" onClick={() => void add()} disabled={adding || addMutation.isPending || !filename.trim()}>{adding || addMutation.isPending ? <Loader2 className="animate-spin" size={15} /> : <Upload size={15} />} 添加来源</button></div>
                <div className="grid gap-3 md:grid-cols-[150px_minmax(0,1fr)]">
                    <label className="text-xs text-muted">类型<select value={sourceType} onChange={(event) => setSourceType(event.target.value)} className="field mt-1 w-full"><option value="markdown">Markdown</option><option value="code">代码</option><option value="url">URL</option><option value="file">本地文件</option></select></label>
                    <label className="text-xs text-muted">文件名或路径<input value={filename} onChange={(event) => setFilename(event.target.value)} className="field mt-1 w-full" placeholder="notes.md" /></label>
                </div>
                {sourceType === "url" ? <label className="mt-3 block text-xs text-muted">来源 URL<input value={sourceUrl} onChange={(event) => setSourceUrl(event.target.value)} className="field mt-1 w-full" placeholder="https://..." /></label> : null}
                <label className="mt-3 block text-xs text-muted">内容（可选）<textarea value={content} onChange={(event) => setContent(event.target.value)} className="field mt-1 min-h-24 w-full resize-y font-mono" placeholder="粘贴 Markdown 或代码内容" /></label>
            </div>
            <div className="surface overflow-hidden rounded-lg">
                <div className="flex items-center justify-between border-b border-line px-4 py-3"><h2 className="text-sm font-semibold text-ink">已登记来源</h2><button type="button" className="action-secondary" disabled={rescanMutation.isPending} onClick={() => void rescanMutation.mutate()}>{rescanMutation.isPending ? <Loader2 className="animate-spin" size={14} /> : <RefreshCw size={14} />} 重新扫描</button></div>
                {sourcesQuery.error ? <div className="flex items-center justify-between gap-2 border-b border-line bg-danger-soft px-4 py-2 text-xs text-danger" role="alert"><span>{errorMessage(sourcesQuery.error)}</span><button type="button" className="action-secondary" onClick={() => void sourcesQuery.refetch()}><RefreshCw size={13} /> 重试</button></div> : null}
                {loading ? (
                    <div className="p-8 text-center text-sm text-muted"><Loader2 className="mx-auto mb-2 animate-spin" size={17} />读取中</div>
                ) : sources.length ? (
                    <div className="divide-y divide-line">
                        {sources.map((source) => {
                            const active = activeSourceIds.has(source.id);
                            return (
                                <div key={source.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
                                    <div className="flex min-w-0 flex-1 items-center gap-3">
                                        <GitBranch className="shrink-0 text-accent" size={17} />
                                        <div className="min-w-0">
                                            <div className="truncate text-sm font-medium text-ink">{source.filename}</div>
                                            <div className="mt-1 flex flex-wrap gap-2 text-[11px] text-muted">
                                                <span>{source.source_type}</span><span>{source.page_count} 页面</span><span>{shortDate(source.ingested_at)}</span>
                                            </div>
                                        </div>
                                    </div>
                                    <StatusBadge status={source.status === "ingested" ? "success" : source.status === "failed" ? "danger" : "warning"}>
                                        {active ? "processing" : source.status}
                                    </StatusBadge>
                                    <div className="flex items-center gap-1">
                                        <button type="button" className="action-secondary" disabled={ingestingId === source.id || active || ingestMutation.isPending} onClick={() => void ingest(source)}>
                                            {ingestingId === source.id || active ? <Loader2 className="animate-spin" size={14} /> : <RefreshCw size={14} />} 摄入
                                        </button>
                                        <IconButton label={`删除来源 ${source.filename}`} tone="danger" onClick={() => setDeleteTarget(source)}><Trash2 size={15} /></IconButton>
                                    </div>
                                    {source.error_message ? <p className="basis-full text-xs text-danger">{source.error_message}</p> : null}
                                </div>
                            );
                        })}
                    </div>
                ) : <div className="p-10 text-center text-sm text-muted">还没有来源。</div>}
            </div>
            {deleteTarget ? <Modal title="删除来源" description="删除来源记录不会影响其他 Wiki 项目。" onClose={() => setDeleteTarget(null)} footer={<><button type="button" className="action-secondary" disabled={deleteMutation.isPending} onClick={() => setDeleteTarget(null)}>取消</button><button type="button" className="action-danger" disabled={deleteMutation.isPending} onClick={() => void remove()}>{deleteMutation.isPending ? "删除中..." : "删除来源"}</button></>}><p className="text-sm text-muted">确定删除 <strong className="text-ink">{deleteTarget.filename}</strong>？</p></Modal> : null}
        </div>
    );
}

function WikiSearch({ project }: { project: WikiProject }) {
    const [query, setQuery] = useState("");
    const [debouncedQuery, setDebouncedQuery] = useState("");
    const [offset, setOffset] = useState(0);
    const pageSize = 20;

    useEffect(() => {
        setQuery("");
        setDebouncedQuery("");
        setOffset(0);
    }, [project.id]);

    useEffect(() => {
        const normalized = query.trim();
        if (!normalized) {
            setDebouncedQuery("");
            setOffset(0);
            return;
        }
        const timer = window.setTimeout(() => {
            setOffset(0);
            setDebouncedQuery(normalized);
        }, 280);
        return () => window.clearTimeout(timer);
    }, [query]);

    const searchQuery = useQuery({
        queryKey: queryKeys.wikiSearch(project.id, debouncedQuery, offset),
        queryFn: () => wikiApi.searchPage(project.id, debouncedQuery, pageSize, offset),
        enabled: debouncedQuery.length > 0,
        staleTime: 30_000,
    });
    const page = searchQuery.data ?? null;
    const pendingInput = query.trim().length > 0 && query.trim() !== debouncedQuery;
    const loading = pendingInput || searchQuery.isFetching;
    const runSearch = () => {
        const normalized = query.trim();
        setOffset(0);
        setDebouncedQuery(normalized);
    };

    const currentOffset = page?.offset ?? 0;
    const hasPrevious = currentOffset > 0;
    const hasNext = page ? currentOffset + page.results.length < page.total : false;

    return (
        <div className="surface rounded-lg p-4">
            <div className="flex gap-2">
                <input
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    onKeyDown={(event) => {
                        if (event.key === "Enter") {
                            event.preventDefault();
                            runSearch();
                        }
                    }}
                    className="field min-w-0 flex-1"
                    placeholder="搜索标题、路径或页面内容"
                    aria-label="搜索 Wiki"
                />
                <button type="button" className="action-primary" onClick={runSearch} disabled={loading || !query.trim()}>
                    {loading ? <Loader2 className="animate-spin" size={15} /> : <Search size={15} />} 搜索
                </button>
            </div>
            {searchQuery.error ? (
                <div className="mt-3 flex items-center justify-between gap-2 rounded-md bg-danger-soft px-3 py-2 text-xs text-danger" role="alert">
                    <span>{errorMessage(searchQuery.error)}</span>
                    <button type="button" className="action-secondary" onClick={() => void searchQuery.refetch()}><RefreshCw size={13} /> 重试</button>
                </div>
            ) : null}
            <div className="mt-4 divide-y divide-line">
                {page?.results.length ? page.results.map((result) => (
                    <article key={result.page_id} className="py-3">
                        <div className="flex items-center justify-between gap-3">
                            <h3 className="text-sm font-medium text-ink">{result.title}</h3>
                            <span className="font-mono text-[11px] text-accent">{result.score.toFixed(2)}</span>
                        </div>
                        <p className="mt-1 font-mono text-[11px] text-muted">{result.path}</p>
                        {result.snippet ? <p className="mt-2 text-sm leading-6 text-muted">{result.snippet}</p> : null}
                    </article>
                )) : <p className="py-12 text-center text-sm text-muted">{query.trim() ? (loading ? "搜索中..." : "没有匹配的页面。") : "输入关键词开始搜索。"}</p>}
            </div>
            {page && page.total > 0 ? (
                <div className="mt-3 flex items-center justify-between border-t border-line pt-3 text-xs text-muted">
                    <span>共 {page.total} 个结果 · {page.offset + 1}-{Math.min(page.offset + page.results.length, page.total)}</span>
                    <div className="flex gap-2">
                        <button type="button" className="action-secondary" disabled={!hasPrevious || loading} onClick={() => setOffset(Math.max(0, currentOffset - pageSize))}>上一页</button>
                        <button type="button" className="action-secondary" disabled={!hasNext || loading} onClick={() => setOffset(currentOffset + pageSize)}>下一页</button>
                    </div>
                </div>
            ) : null}
        </div>
    );
}

function WikiGraph({ project }: { project: WikiProject }) {
    const graphQuery = useQuery({
        queryKey: queryKeys.wikiGraph(project.id),
        queryFn: () => wikiApi.getGraph(project.id),
    });
    const graph = graphQuery.data;
    if (graphQuery.isPending) return <div className="surface flex min-h-48 items-center justify-center rounded-lg text-sm text-muted"><Loader2 className="mr-2 animate-spin" size={17} />正在构建关系图谱</div>;
    if (graphQuery.error) return <div className="surface rounded-lg p-12 text-center text-sm text-danger"><p>{errorMessage(graphQuery.error)}</p><button type="button" className="action-secondary mt-4" onClick={() => void graphQuery.refetch()}><RefreshCw size={13} /> 重试</button></div>;
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
