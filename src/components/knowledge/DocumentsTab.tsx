import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, Clock, FileText, Loader2, RefreshCw, Trash2, Upload, XCircle } from "lucide-react";
import { Modal } from "../ui";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { kbApi } from "../../lib/api";
import { errorMessage, queryKeys } from "../../lib/query";
import type { KbDocument, KnowledgeBase } from "../../types";
import { fileToBase64, formatSize, kbErrorMessage } from "./helpers";

export function DocumentsTab({ kb, onRefresh }: { kb: KnowledgeBase; onRefresh: () => void }) {
  const queryClient = useQueryClient();
  const docsQuery = useQuery({
    queryKey: queryKeys.kbDocuments(kb.id),
    queryFn: () => kbApi.getDocuments(kb.id),
    staleTime: 1_000,
    refetchInterval: ({ state }) => state.error ? 8_000 : 3_000,
  });
  const docs = docsQuery.data ?? [];
  const [uploadingCount, setUploadingCount] = useState(0);
  const [uploadTotal, setUploadTotal] = useState(0);
  const [errorNotices, setErrorNotices] = useState<{ doc_id: string; filename: string; error: string }[]>([]);
  const [progressMap, setProgressMap] = useState<Record<string, { stage: string; progress: number; detail: string }>>({});
  const [deleteTarget, setDeleteTarget] = useState<KbDocument | null>(null);

  const addErrorNotice = (docId: string, filename: string, error: unknown) => {
    setErrorNotices((current) => [
      ...current.filter((notice) => notice.doc_id !== docId),
      { doc_id: docId, filename, error: kbErrorMessage(error) },
    ]);
  };

  const refreshDocs = () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.kbDocuments(kb.id) });
  };

  const deleteMutation = useMutation({
    mutationFn: ({ docId, kbId }: { docId: string; kbId: string }) => kbApi.deleteDocument(docId, kbId),
    onSuccess: () => {
      setDeleteTarget(null);
      refreshDocs();
      onRefresh();
    },
    onError: (mutationError) => {
      if (deleteTarget) addErrorNotice(deleteTarget.id, deleteTarget.filename, `删除失败：${kbErrorMessage(mutationError)}`);
    },
  });
  const reindexMutation = useMutation({
    mutationFn: (docId: string) => kbApi.reindexDocument(docId),
    onSuccess: () => refreshDocs(),
    onError: (mutationError, docId) => {
      const document = docs.find((doc) => doc.id === docId);
      addErrorNotice(docId, document?.filename ?? "文档", `重新索引失败：${kbErrorMessage(mutationError)}`);
    },
  });
  const deletingDocumentId = deleteTarget && deleteMutation.isPending ? deleteTarget.id : null;

  useTauriEvent<{ doc_id: string; kb_id: string; filename: string; error: string }>(
    "kb-document-error",
    (payload) => {
      if (payload.kb_id !== kb.id) return;
      setErrorNotices((prev) => [...prev, payload]);
      setProgressMap((prev) => {
        const next = { ...prev };
        delete next[payload.doc_id];
        return next;
      });
      setTimeout(() => {
        setErrorNotices((prev) => prev.filter((notice) => notice.doc_id !== payload.doc_id));
      }, 8000);
      refreshDocs();
      onRefresh();
    },
  );

  useTauriEvent<{ doc_id: string; kb_id: string; filename: string; stage: string; progress: number; detail: string }>(
    "kb-document-progress",
    (payload) => {
      if (payload.kb_id !== kb.id) return;
      if (payload.stage === "done") {
        setProgressMap((prev) => {
          const next = { ...prev };
          delete next[payload.doc_id];
          return next;
        });
        refreshDocs();
        onRefresh();
      } else {
        setProgressMap((prev) => ({
          ...prev,
          [payload.doc_id]: { stage: payload.stage, progress: payload.progress, detail: payload.detail },
        }));
      }
    },
  );

  const handleUploadBatch = async (files: File[]) => {
    if (files.length === 0) return;
    setUploadTotal(files.length);
    setUploadingCount(0);
    for (const file of files) {
      try {
        const content = await fileToBase64(file);
        await kbApi.uploadDocument({
          kb_id: kb.id,
          filename: file.name,
          content,
        });
      } catch (e) {
        console.error(`Upload failed for ${file.name}:`, e);
        addErrorNotice(`upload-${file.name}-${Date.now()}`, file.name, e);
      }
      setUploadingCount(prev => prev + 1);
    }
    setUploadTotal(0);
    setUploadingCount(0);
    refreshDocs();
    onRefresh();
  };

  const handleDelete = async () => {
    if (!deleteTarget) return;
    await deleteMutation.mutateAsync({ docId: deleteTarget.id, kbId: kb.id });
  };

  const handleReindex = async (docId: string) => {
    await reindexMutation.mutateAsync(docId);
  };

  return (
    <div className="space-y-4">
      {/* Upload zone */}
      <label className="flex cursor-pointer items-center justify-center rounded-2xl border-2 border-dashed border-slate-300 bg-white px-6 py-8 transition-colors hover:border-blue-400 hover:bg-blue-50/30">
        <input
          type="file"
          className="hidden"
          multiple
          accept=".md,.txt,.json,.yaml,.yml,.rs,.ts,.tsx,.js,.py,.go,.java,.c,.cpp,.h,.sh,.toml,.xml,.html,.css,.pdf"
          onChange={(e) => {
            const files = Array.from(e.target.files || []);
            if (files.length > 0) void handleUploadBatch(files);
            e.target.value = "";
          }}
          disabled={uploadTotal > 0}
        />
        {uploadTotal > 0 ? (
          <div className="flex items-center gap-2 text-sm text-blue-600">
            <Loader2 className="h-5 w-5 animate-spin" />
            上传中 {uploadingCount}/{uploadTotal}...
          </div>
        ) : (
          <div className="flex flex-col items-center gap-2 text-sm text-slate-500">
            <Upload className="h-6 w-6" />
            <span>点击或拖拽上传文件到知识库（支持多选）</span>
            <span className="text-xs text-slate-400">支持 md/txt/code/json/yaml/pdf</span>
          </div>
        )}
      </label>

      {/* Error notices */}
      {errorNotices.length > 0 && (
        <div className="space-y-2">
          {errorNotices.map((notice) => (
            <div
              key={notice.doc_id}
              className="flex items-start gap-3 rounded-xl border border-red-200 bg-red-50 px-4 py-3"
            >
              <XCircle className="mt-0.5 h-5 w-5 shrink-0 text-red-500" />
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium text-red-800">
                  {notice.filename} 处理失败
                </div>
                <div className="mt-0.5 text-xs text-red-600">{notice.error}</div>
              </div>
              <button
                type="button"
                aria-label={`关闭 ${notice.filename} 错误提示`}
                onClick={() =>
                  setErrorNotices((prev) =>
                    prev.filter((n) => n.doc_id !== notice.doc_id)
                  )
                }
                className="shrink-0 rounded-lg p-1 text-red-400 hover:bg-red-100 hover:text-red-600"
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Documents list */}
      {docsQuery.isPending && docs.length === 0 ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="h-6 w-6 animate-spin text-slate-400" />
        </div>
      ) : docsQuery.error && docs.length === 0 ? (
        <div className="surface empty-state rounded-2xl" role="alert">
          <XCircle className="h-8 w-8 text-red-400" />
          <p className="text-sm text-red-600">{errorMessage(docsQuery.error)}</p>
          <button type="button" className="action-secondary" onClick={() => void docsQuery.refetch()}>重试</button>
        </div>
      ) : docs.length === 0 ? (
        <div className="surface empty-state rounded-2xl">
          <FileText className="h-8 w-8 text-slate-300" />
          <p className="text-sm text-slate-500">暂无文档</p>
        </div>
      ) : (
        <div className="space-y-2">
          {docs.map((doc) => {
            const prog = progressMap[doc.id];
            return (
            <div
              key={doc.id}
              className="surface flex items-center gap-3 rounded-xl px-4 py-3"
            >
              <DocStatusIcon status={prog ? "processing" : doc.status} />

              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-medium text-slate-900">
                    {doc.filename}
                  </span>
                  <span className="rounded bg-slate-100 px-1.5 py-0.5 text-[10px] text-slate-500">
                    {doc.file_type}
                  </span>
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
                    <span>{formatSize(doc.file_size)}</span>
                    {doc.chunk_count > 0 && <span>{doc.chunk_count} 切片</span>}
                    {doc.token_count > 0 && <span>{doc.token_count} tokens</span>}
                    {doc.error_message && (
                      <span className="text-red-500" title={doc.error_message}>
                        {doc.error_message.slice(0, 50)}
                      </span>
                    )}
                  </div>
                )}
              </div>

              <div className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => void handleReindex(doc.id)}
                  disabled={reindexMutation.isPending && reindexMutation.variables === doc.id}
                  className="rounded-lg p-1.5 text-slate-400 hover:bg-slate-100 hover:text-slate-600"
                  title="重新索引"
                  aria-label={`重新索引 ${doc.filename}`}
                >
                  <RefreshCw size={15} />
                </button>
                <button
                  type="button"
                  onClick={() => setDeleteTarget(doc)}
                  className="rounded-lg p-1.5 text-slate-400 hover:bg-red-50 hover:text-red-500"
                  title="删除"
                  aria-label={`删除文档 ${doc.filename}`}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            </div>
            );
          })}
        </div>
      )}

      {deleteTarget && (
        <Modal
          title="删除文档"
          description="文档及其切片会从知识库中移除，此操作无法恢复。"
          onClose={() => { if (!deletingDocumentId) setDeleteTarget(null); }}
          footer={(
            <>
              <button type="button" className="action-secondary" disabled={Boolean(deletingDocumentId)} onClick={() => setDeleteTarget(null)}>取消</button>
              <button type="button" className="button-danger" disabled={Boolean(deletingDocumentId)} onClick={() => void handleDelete()}>
                {deletingDocumentId ? <Loader2 className="animate-spin" size={15} /> : <Trash2 size={15} />}
                删除文档
              </button>
            </>
          )}
        >
          <p className="break-all text-sm text-muted">{deleteTarget.filename}</p>
        </Modal>
      )}
    </div>
  );
}

function DocStatusIcon({ status }: { status: string }) {
  switch (status) {
    case "ready":
      return <CheckCircle2 className="h-5 w-5 shrink-0 text-emerald-500" />;
    case "processing":
      return <Loader2 className="h-5 w-5 shrink-0 animate-spin text-blue-500" />;
    case "failed":
      return <XCircle className="h-5 w-5 shrink-0 text-red-500" />;
    case "pending":
      return <Clock className="h-5 w-5 shrink-0 text-slate-400" />;
    default:
      return <FileText className="h-5 w-5 shrink-0 text-slate-400" />;
  }
}
