import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, Clock, FileText, FolderInput, FolderOpen, GitBranch, Link, Loader2, Plus, Trash2, XCircle } from "lucide-react";
import { Modal } from "../ui";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { kbApi } from "../../lib/api";
import { queryKeys } from "../../lib/query";
import type { KbSource, KnowledgeBase } from "../../types";
import { kbErrorMessage } from "./helpers";

export function SourcesTab({ kb, onRefresh }: { kb: KnowledgeBase; onRefresh: () => void }) {
  const queryClient = useQueryClient();
  const sourcesQuery = useQuery({
    queryKey: queryKeys.kbSources(kb.id),
    queryFn: () => kbApi.getSources(kb.id),
    refetchInterval: ({ state }) => (
      state.data?.some((source) => source.status === "pending" || source.status === "processing")
        ? 3_000
        : false
    ),
  });
  const sources = sourcesQuery.data ?? [];
  const loading = sourcesQuery.isPending;
  const [showImport, setShowImport] = useState(false);
  const [progressMap, setProgressMap] = useState<Record<string, { progress: number; detail: string }>>({});
  const [deleteTarget, setDeleteTarget] = useState<KbSource | null>(null);
  const [error, setError] = useState("");

  const refreshSources = () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.kbSources(kb.id) });
  };

  useTauriEvent<{ kb_id: string; source_id: string; progress: number; detail: string }>(
    "kb-import-progress",
    (payload) => {
      if (payload.kb_id !== kb.id) return;
      if (payload.progress >= 100) {
        setProgressMap((prev) => {
          const next = { ...prev };
          delete next[payload.source_id];
          return next;
        });
        refreshSources();
        onRefresh();
      } else {
        setProgressMap((prev) => ({
          ...prev,
          [payload.source_id]: { progress: payload.progress, detail: payload.detail },
        }));
      }
    },
  );

  const deleteMutation = useMutation({
    mutationFn: (source: KbSource) => kbApi.deleteSource(source.id, kb.id),
    onSuccess: async () => {
      setDeleteTarget(null);
      setError("");
      await queryClient.invalidateQueries({ queryKey: queryKeys.kbSources(kb.id) });
      onRefresh();
    },
    onError: (mutationError) => setError(`删除来源失败：${kbErrorMessage(mutationError)}`),
  });

  const handleDelete = async () => {
    if (!deleteTarget) return;
    setError("");
    try { await deleteMutation.mutateAsync(deleteTarget); } catch { /* Mutation callback reports the error. */ }
  };

  return (
    <div className="space-y-4">
      {error || sourcesQuery.error ? <div className="kb-notice kb-notice-warning" role="alert">{error || kbErrorMessage(sourcesQuery.error)}</div> : null}
      <div className="flex justify-end">
        <button type="button" onClick={() => setShowImport(true)} className="action-primary">
          <Plus size={16} />
          导入来源
        </button>
      </div>

      {loading && sources.length === 0 ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="h-6 w-6 animate-spin text-slate-400" />
        </div>
      ) : sources.length === 0 ? (
        <div className="surface empty-state rounded-2xl">
          <GitBranch className="h-8 w-8 text-slate-300" />
          <p className="text-sm text-slate-500">暂无导入来源</p>
          <p className="text-xs text-slate-400 mt-1">从 Git 仓库、URL 或本地目录导入文档</p>
        </div>
      ) : (
        <div className="space-y-2">
          {sources.map((src) => {
            const prog = progressMap[src.id];
            return (
              <div key={src.id} className="surface flex items-center gap-3 rounded-xl px-4 py-3">
                <SourceIcon type={src.source_type} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-medium text-slate-900">
                      {src.source_url || src.source_path || src.source_type}
                    </span>
                    {src.branch && src.source_type === "git" && (
                      <span className="rounded bg-slate-100 px-1.5 py-0.5 text-[10px] text-slate-500">
                        {src.branch}
                      </span>
                    )}
                  </div>
                  {prog ? (
                    <div className="mt-1.5">
                      <div className="flex items-center gap-2">
                        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-slate-200">
                          <div
                            className="h-full rounded-full bg-blue-500 transition-all duration-300"
                            style={{ width: `${prog.progress}%` }}
                          />
                        </div>
                        <span className="shrink-0 text-[11px] text-blue-600">
                          {prog.detail} · {prog.progress}%
                        </span>
                      </div>
                    </div>
                  ) : (
                    <div className="mt-1 flex items-center gap-3 text-xs text-slate-500">
                      <SourceStatusBadge status={src.status} />
                      {src.file_count > 0 && <span>{src.file_count} 文件</span>}
                      {src.error && (
                        <span className="text-red-500 truncate" title={src.error}>
                          {src.error.slice(0, 60)}
                        </span>
                      )}
                    </div>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => setDeleteTarget(src)}
                  className="rounded-lg p-1.5 text-slate-400 hover:bg-red-50 hover:text-red-500"
                  title="删除"
                  aria-label={`删除来源 ${src.source_url || src.source_path || src.source_type}`}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            );
          })}
        </div>
      )}

      {showImport && (
        <ImportSourceModal
          kbId={kb.id}
          onClose={() => setShowImport(false)}
          onImported={async () => {
            setShowImport(false);
            await queryClient.invalidateQueries({ queryKey: queryKeys.kbSources(kb.id) });
            onRefresh();
          }}
        />
      )}

      {deleteTarget && (
        <Modal
          title="删除来源"
          description="关联文档会保留，但不会再标记为来自该来源。"
          onClose={() => { if (!deleteMutation.isPending) setDeleteTarget(null); }}
          footer={(
            <>
              <button type="button" className="action-secondary" disabled={deleteMutation.isPending} onClick={() => setDeleteTarget(null)}>取消</button>
              <button type="button" className="button-danger" disabled={deleteMutation.isPending} onClick={() => void handleDelete()}>
                {deleteMutation.isPending ? <Loader2 className="animate-spin" size={15} /> : <Trash2 size={15} />}
                删除来源
              </button>
            </>
          )}
        >
          <p className="break-all text-sm text-muted">{deleteTarget.source_url || deleteTarget.source_path || deleteTarget.source_type}</p>
        </Modal>
      )}
    </div>
  );
}

function SourceIcon({ type }: { type: string }) {
  const cls = "h-5 w-5 shrink-0 text-slate-400";
  switch (type) {
    case "git":
      return <GitBranch className={cls} />;
    case "url":
      return <Link className={cls} />;
    case "local_dir":
      return <FolderOpen className={cls} />;
    default:
      return <FileText className={cls} />;
  }
}

function SourceStatusBadge({ status }: { status: string }) {
  switch (status) {
    case "done":
      return <span className="flex items-center gap-1 text-emerald-600"><CheckCircle2 size={12} /> 完成</span>;
    case "processing":
      return <span className="flex items-center gap-1 text-blue-600"><Loader2 size={12} className="animate-spin" /> 处理中</span>;
    case "error":
      return <span className="flex items-center gap-1 text-red-500"><XCircle size={12} /> 失败</span>;
    default:
      return <span className="flex items-center gap-1 text-slate-400"><Clock size={12} /> 等待中</span>;
  }
}


function ImportSourceModal({
  kbId,
  onClose,
  onImported,
}: {
  kbId: string;
  onClose: () => void;
  onImported: () => void;
}) {
  const [sourceType, setSourceType] = useState<"git" | "url" | "local_dir">("git");
  const [repoUrl, setRepoUrl] = useState("");
  const [branch, setBranch] = useState("main");
  const [token, setToken] = useState("");
  const [url, setUrl] = useState("");
  const [dirPath, setDirPath] = useState("");
  const [excludedDirs, setExcludedDirs] = useState("");
  const [includedFiles, setIncludedFiles] = useState("");
  const [maxFileSize, setMaxFileSize] = useState(1);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleImport = async () => {
    setImporting(true);
    setError(null);
    try {
      const input: Record<string, unknown> = {
        source_type: sourceType,
        excluded_dirs: excludedDirs ? excludedDirs.split(",").map((s) => s.trim()) : [],
        included_files: includedFiles ? includedFiles.split(",").map((s) => s.trim()) : [],
        max_file_size: maxFileSize * 1024 * 1024,
      };

      if (sourceType === "git") {
        if (!repoUrl.trim()) { setError("请输入仓库 URL"); setImporting(false); return; }
        input.repo_url = repoUrl.trim();
        input.branch = branch.trim() || "main";
        if (token.trim()) input.token = token.trim();
      } else if (sourceType === "url") {
        if (!url.trim()) { setError("请输入 URL"); setImporting(false); return; }
        input.url = url.trim();
      } else if (sourceType === "local_dir") {
        if (!dirPath.trim()) { setError("请输入目录路径"); setImporting(false); return; }
        input.dir_path = dirPath.trim();
      }

      await kbApi.importSource(kbId, input as Parameters<typeof kbApi.importSource>[1]);
      onImported();
    } catch (e) {
      setError(kbErrorMessage(e));
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30" onClick={onClose}>
      <div className="w-full max-w-lg rounded-2xl bg-white p-6 shadow-xl" onClick={(e) => e.stopPropagation()}>
        <h3 className="text-lg font-semibold text-slate-900">导入来源</h3>

        {/* Source type tabs */}
        <div className="mt-4 flex gap-2">
          {([
            { key: "git" as const, label: "Git 仓库", icon: GitBranch },
            { key: "url" as const, label: "URL", icon: Link },
            { key: "local_dir" as const, label: "本地目录", icon: FolderOpen },
          ]).map(({ key, label, icon: Icon }) => (
            <button
              key={key}
              onClick={() => setSourceType(key)}
              className={`flex items-center gap-2 rounded-xl px-4 py-2.5 text-sm font-medium transition-all ${
                sourceType === key
                  ? "border border-blue-100 bg-white text-slate-900 shadow-sm"
                  : "text-slate-500 hover:bg-white/70"
              }`}
            >
              <Icon size={15} />
              {label}
            </button>
          ))}
        </div>

        <div className="mt-4 space-y-4">
          {sourceType === "git" && (
            <>
              <div>
                <label className="mb-1 block text-sm font-medium text-slate-700">仓库 URL</label>
                <input
                  type="text"
                  value={repoUrl}
                  onChange={(e) => setRepoUrl(e.target.value)}
                  placeholder="https://github.com/owner/repo"
                  className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="mb-1 block text-sm font-medium text-slate-700">分支</label>
                  <input
                    type="text"
                    value={branch}
                    onChange={(e) => setBranch(e.target.value)}
                    placeholder="main"
                    className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                  />
                </div>
                <div>
                  <label className="mb-1 block text-sm font-medium text-slate-700">Access Token（可选）</label>
                  <input
                    type="password"
                    value={token}
                    onChange={(e) => setToken(e.target.value)}
                    placeholder="私有仓库需要"
                    className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                  />
                </div>
              </div>
            </>
          )}

          {sourceType === "url" && (
            <div>
              <label className="mb-1 block text-sm font-medium text-slate-700">URL</label>
              <input
                type="text"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://example.com/doc.md"
                className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
              />
            </div>
          )}

          {sourceType === "local_dir" && (
            <div>
              <label className="mb-1 block text-sm font-medium text-slate-700">目录路径</label>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={dirPath}
                  onChange={(e) => setDirPath(e.target.value)}
                  placeholder="/path/to/project/docs"
                  className="flex-1 rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                />
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      const { open } = await import("@tauri-apps/plugin-dialog");
                      const selected = await open({
                        directory: true,
                        multiple: false,
                        title: "选择导入目录",
                      });
                      if (typeof selected === "string") {
                        setDirPath(selected);
                      }
                    } catch {
                      // 对话框取消或不可用，忽略
                    }
                  }}
                  className="flex items-center gap-1.5 rounded-xl border border-slate-200 bg-slate-50 px-3 py-2 text-sm font-medium text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900"
                >
                  <FolderInput size={15} />
                  浏览
                </button>
              </div>
            </div>
          )}

          {/* Common filter options */}
          <div className="rounded-xl bg-slate-50 p-3 space-y-3">
            <div className="text-xs font-semibold text-slate-500">过滤选项（可选）</div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="mb-1 block text-xs text-slate-600">排除目录（逗号分隔）</label>
                <input
                  type="text"
                  value={excludedDirs}
                  onChange={(e) => setExcludedDirs(e.target.value)}
                  placeholder="tests, examples, docs"
                  className="w-full rounded-lg border border-slate-200 px-2.5 py-1.5 text-xs outline-none focus:border-blue-400"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs text-slate-600">包含文件类型（逗号分隔，空=全部）</label>
                <input
                  type="text"
                  value={includedFiles}
                  onChange={(e) => setIncludedFiles(e.target.value)}
                  placeholder="md, rs, ts, py"
                  className="w-full rounded-lg border border-slate-200 px-2.5 py-1.5 text-xs outline-none focus:border-blue-400"
                />
              </div>
            </div>
            <div>
              <label className="mb-1 block text-xs text-slate-600">最大文件大小 (MB)</label>
              <input
                type="number"
                value={maxFileSize}
                onChange={(e) => setMaxFileSize(Number(e.target.value) || 1)}
                min={0.1}
                step={0.1}
                className="w-24 rounded-lg border border-slate-200 px-2.5 py-1.5 text-xs outline-none focus:border-blue-400"
              />
            </div>
          </div>

          {error && (
            <div className="rounded-lg bg-red-50 p-3 text-sm text-red-600">{error}</div>
          )}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button onClick={onClose} className="rounded-xl px-4 py-2 text-sm text-slate-500 hover:bg-slate-100">
            取消
          </button>
          <button
            onClick={handleImport}
            disabled={importing}
            className="action-primary disabled:opacity-50"
          >
            {importing ? "导入中..." : "开始导入"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── Index Tab ─────────────────────────────────────────────────────────
