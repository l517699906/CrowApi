import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, Clock, Database, Loader2, Settings as SettingsIcon, Trash2, XCircle } from "lucide-react";
import { Modal } from "../ui";
import { useBackgroundTasks } from "../../hooks/useBackgroundTasks";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { kbApi } from "../../lib/api";
import { queryKeys } from "../../lib/query";
import type { KnowledgeBase } from "../../types";
import { kbErrorMessage } from "./helpers";

export function IndexTab({ kb }: { kb: KnowledgeBase }) {
  const queryClient = useQueryClient();
  const taskQuery = useBackgroundTasks({
    domain: "knowledge",
    resourceType: "knowledge_base",
    resourceId: kb.id,
    limit: 30,
  });
  const latestIndexTask = useMemo(() => (
    (taskQuery.data ?? []).find((task) => task.taskType === "build_index")
  ), [taskQuery.data]);
  const activeIndexTask = latestIndexTask
    && (latestIndexTask.status === "pending" || latestIndexTask.status === "running")
    ? latestIndexTask
    : null;
  const indexQuery = useQuery({
    queryKey: queryKeys.kbIndex(kb.id),
    queryFn: () => kbApi.getIndexStatus(kb.id),
    refetchInterval: activeIndexTask ? 3_000 : false,
  });
  const indexMeta = indexQuery.data ?? null;
  const [buildMsg, setBuildMsg] = useState("");
  const [eventProgress, setEventProgress] = useState(0);
  const [showDropConfirm, setShowDropConfirm] = useState(false);

  useEffect(() => {
    if (!latestIndexTask || activeIndexTask) return;
    void queryClient.invalidateQueries({ queryKey: queryKeys.kbIndex(kb.id) });
    if (latestIndexTask.status === "failed") {
      setBuildMsg(`构建失败：${latestIndexTask.errorMessage || "后台任务执行失败"}`);
    } else if (latestIndexTask.status === "succeeded") {
      setBuildMsg("");
      setEventProgress(100);
    }
  }, [activeIndexTask, kb.id, latestIndexTask, queryClient]);

  useTauriEvent<{ kb_id: string; status: string; message: string; progress?: number; current?: number; total?: number }>(
    "kb-index-progress",
    (payload) => {
      if (payload.kb_id !== kb.id) return;

      setBuildMsg(payload.message);
      if (payload.status === "ready") {
        setEventProgress(100);
        setBuildMsg("");
        void queryClient.invalidateQueries({ queryKey: queryKeys.kbIndex(kb.id) });
      } else if (payload.status === "error") {
        setEventProgress(0);
      } else if (payload.status === "building") {
        setEventProgress(payload.progress ?? 0);
      }
    },
  );

  const buildMutation = useMutation({
    mutationFn: () => kbApi.buildIndex(kb.id),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.backgroundTasks(
          "knowledge",
          "knowledge_base",
          kb.id,
        ) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.kbIndex(kb.id) }),
      ]);
    },
    onError: (error) => setBuildMsg(`构建失败：${kbErrorMessage(error)}`),
  });
  const dropMutation = useMutation({
    mutationFn: () => kbApi.dropIndex(kb.id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.kbIndex(kb.id) });
      setShowDropConfirm(false);
    },
    onError: (error) => setBuildMsg(`删除索引失败：${kbErrorMessage(error)}`),
  });
  const building = buildMutation.isPending || Boolean(activeIndexTask) || indexMeta?.status === "building";
  const buildProgress = activeIndexTask?.progress ?? eventProgress;
  const dropping = dropMutation.isPending;

  const handleBuild = async () => {
    setEventProgress(0);
    setBuildMsg("正在构建 HNSW 向量索引…");
    try { await buildMutation.mutateAsync(); } catch { /* Mutation callback reports the error. */ }
  };

  const handleDrop = async () => {
    setBuildMsg("");
    try { await dropMutation.mutateAsync(); } catch { /* Mutation callback reports the error. */ }
  };

  if (indexQuery.isPending) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-slate-400" />
      </div>
    );
  }

  if (indexQuery.error) {
    return (
      <div className="surface rounded-xl px-5 py-10 text-center text-sm text-red-600" role="alert">
        <p>{kbErrorMessage(indexQuery.error)}</p>
        <button type="button" className="action-secondary mt-4" onClick={() => void indexQuery.refetch()}>
          重试读取索引状态
        </button>
      </div>
    );
  }

  return (
    <>
      <div className="grid gap-4 lg:grid-cols-2">
      {/* Index status */}
      <div className="surface data-card rounded-2xl">
        <div className="mb-4 flex items-center gap-2">
          <Database size={18} className="text-slate-700" />
          <h3 className="text-sm font-semibold text-slate-900">索引状态</h3>
        </div>
        {indexMeta ? (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-3">
              <div className="rounded-xl bg-slate-50 p-3">
                <div className="text-xs text-slate-500">索引类型</div>
                <div className="text-sm font-medium text-slate-900 mt-1">{indexMeta.index_type || "linear"}</div>
              </div>
              <div className="rounded-xl bg-slate-50 p-3">
                <div className="text-xs text-slate-500">状态</div>
                <div className="text-sm font-medium mt-1">
                  <IndexStatusBadge status={indexMeta.status} />
                </div>
              </div>
              <div className="rounded-xl bg-slate-50 p-3">
                <div className="text-xs text-slate-500">Embedding 维度</div>
                <div className="text-sm font-medium text-slate-900 mt-1">{indexMeta.embedding_dim || "未检测"}</div>
              </div>
              <div className="rounded-xl bg-slate-50 p-3">
                <div className="text-xs text-slate-500">切片数量</div>
                <div className="text-sm font-medium text-slate-900 mt-1">{indexMeta.chunk_count}</div>
              </div>
            </div>
            {indexMeta.built_at && (
              <div className="text-xs text-slate-400">构建时间: {indexMeta.built_at}</div>
            )}
          </div>
        ) : (
          <div className="text-sm text-slate-500">暂无索引信息</div>
        )}
      </div>

      {/* Actions */}
      <div className="surface data-card rounded-2xl">
        <div className="mb-4 flex items-center gap-2">
          <SettingsIcon size={18} className="text-slate-700" />
          <h3 className="text-sm font-semibold text-slate-900">索引操作</h3>
        </div>
        <div className="space-y-3">
          {building && buildMsg && (
            <div className="rounded-lg bg-blue-50 border border-blue-100 px-3 py-2.5 text-xs text-blue-600">
              <div className="flex items-center gap-2 mb-1.5">
                <Loader2 className="h-3 w-3 animate-spin shrink-0" />
                <span>{buildMsg}</span>
              </div>
              {buildProgress >= 0 && buildProgress < 100 && (
                <div className="mt-1 h-1.5 w-full rounded-full bg-blue-100 overflow-hidden">
                  <div
                    className="h-full bg-blue-500 rounded-full transition-all duration-300"
                    style={{ width: `${Math.max(buildProgress, 3)}%` }}
                  />
                </div>
              )}
            </div>
          )}
          {!building && buildMsg && (
            <div className="rounded-lg bg-red-50 border border-red-100 px-3 py-2 text-xs text-red-600">
              {buildMsg}
            </div>
          )}
          <button
            type="button"
            onClick={() => void handleBuild()}
            disabled={building}
            className="action-primary w-full disabled:opacity-50"
          >
            {building ? (
              <><Loader2 className="h-4 w-4 animate-spin" /> 构建中...</>
            ) : (
              <><Database size={16} /> 构建索引</>
            )}
          </button>
          <button
            type="button"
            onClick={() => setShowDropConfirm(true)}
            disabled={dropping || building || !indexMeta || indexMeta.status === "none"}
            className="w-full rounded-xl border border-red-200 px-4 py-2 text-sm text-red-600 hover:bg-red-50 disabled:opacity-50"
          >
            <Trash2 size={16} />
            删除索引
          </button>
          <div className="rounded-lg bg-blue-50 border border-blue-100 px-3 py-2 text-xs text-blue-600">
            ℹ️ 使用 HNSW 图索引，平均查询复杂度 O(log n)。构建后自动用于检索加速。
          </div>
        </div>
      </div>
      </div>

      {showDropConfirm && (
        <Modal
          title="删除向量索引"
          description="删除后，知识库需要重新构建索引才能恢复向量检索。"
          onClose={() => { if (!dropping) setShowDropConfirm(false); }}
          footer={(
            <>
              <button type="button" className="action-secondary" disabled={dropping} onClick={() => setShowDropConfirm(false)}>取消</button>
              <button type="button" className="button-danger" disabled={dropping} onClick={() => void handleDrop()}>
                {dropping ? <Loader2 className="animate-spin" size={15} /> : <Trash2 size={15} />}
                删除索引
              </button>
            </>
          )}
        >
          <p className="text-sm text-muted">当前索引包含 {indexMeta?.chunk_count ?? 0} 个切片。</p>
        </Modal>
      )}
    </>
  );
}

// ─── Documents Tab ────────────────────────────────────────────────────────


// ─── Search Tab ──────────────────────────────────────────────────────────


// ─── Ask Tab (RAG) ──────────────────────────────────────────────────────

function IndexStatusBadge({ status }: { status: string }) {
  switch (status) {
    case "ready":
      return <span className="flex items-center gap-1 text-emerald-600"><CheckCircle2 size={12} /> 就绪</span>;
    case "building":
      return <span className="flex items-center gap-1 text-blue-600"><Loader2 size={12} className="animate-spin" /> 构建中</span>;
    case "error":
      return <span className="flex items-center gap-1 text-red-500"><XCircle size={12} /> 失败</span>;
    case "none":
      return <span className="flex items-center gap-1 text-slate-400"><Clock size={12} /> 未构建</span>;
    default:
      return <span className="flex items-center gap-1 text-slate-400"><Clock size={12} /> {status}</span>;
  }
}
