import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, CheckCircle2, ChevronDown, Loader2, Settings as SettingsIcon } from "lucide-react";
import { channelApi, kbApi } from "../../lib/api";
import { queryKeys } from "../../lib/query";
import type { KnowledgeBase } from "../../types";
import { kbErrorMessage } from "./helpers";

export function SettingsTab({ kb, onRefresh }: { kb: KnowledgeBase; onRefresh: () => void }) {
  const queryClient = useQueryClient();
  const channelsQuery = useQuery({
    queryKey: queryKeys.channels,
    queryFn: channelApi.getAll,
  });
  const channels = channelsQuery.data ?? [];
  const [name, setName] = useState(kb.name);
  const [description, setDescription] = useState(kb.description || "");
  const [embeddingModel, setEmbeddingModel] = useState(kb.embedding_model || "text-embedding-3-small");
  const [embeddingChannelId, setEmbeddingChannelId] = useState(kb.embedding_channel_id || "");
  const [status, setStatus] = useState(kb.status);
  const [mcpEnabled, setMcpEnabled] = useState(kb.mcp_enabled ?? 1);
  const [chunkSize, setChunkSize] = useState(kb.chunk_size || 512);
  const [chunkOverlap, setChunkOverlap] = useState(kb.chunk_overlap || 64);
  const [embeddingBatchSize, setEmbeddingBatchSize] = useState(kb.embedding_batch_size || 32);
  const [excludedDirs, setExcludedDirs] = useState(kb.excluded_dirs || "");
  const [excludedFiles, setExcludedFiles] = useState(kb.excluded_files || "");
  const [includedFiles, setIncludedFiles] = useState(kb.included_files || "");
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState("");
  const [showChannelPicker, setShowChannelPicker] = useState(false);

  useEffect(() => {
    setName(kb.name);
    setDescription(kb.description || "");
    setEmbeddingModel(kb.embedding_model || "text-embedding-3-small");
    setEmbeddingChannelId(kb.embedding_channel_id || "");
    setStatus(kb.status);
    setMcpEnabled(kb.mcp_enabled ?? 1);
    setChunkSize(kb.chunk_size || 512);
    setChunkOverlap(kb.chunk_overlap || 64);
    setEmbeddingBatchSize(kb.embedding_batch_size || 32);
    setExcludedDirs(kb.excluded_dirs || "");
    setExcludedFiles(kb.excluded_files || "");
    setIncludedFiles(kb.included_files || "");
    setSaved(false);
    setSaveError("");
  }, [kb.id]);

  const activeChannels = channels.filter(c => c.status === 1);
  const selectedEmbeddingChannel = activeChannels.find(c => c.id === embeddingChannelId);

  const saveMutation = useMutation({
    mutationFn: () => kbApi.update(kb.id, {
        name: name.trim(),
        description: description.trim(),
        embedding_model: embeddingModel.trim() || undefined,
        embedding_channel_id: embeddingChannelId,
        status,
        mcp_enabled: mcpEnabled,
        chunk_size: chunkSize,
        chunk_overlap: chunkOverlap,
        embedding_batch_size: embeddingBatchSize,
        excluded_dirs: excludedDirs,
        excluded_files: excludedFiles,
        included_files: includedFiles,
      }),
    onSuccess: async (updated) => {
      queryClient.setQueryData<KnowledgeBase[]>(queryKeys.knowledgeBases, (current) => (
        (current ?? []).map((item) => item.id === updated.id ? updated : item)
      ));
      queryClient.setQueryData(queryKeys.knowledgeBase(updated.id), updated);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.kbIndex(updated.id) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.kbTags(updated.id) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.backgroundTasks(
          "knowledge",
          "knowledge_base",
          updated.id,
        ) }),
      ]);
      setSaved(true);
      onRefresh();
      window.setTimeout(() => setSaved(false), 2000);
    },
    onError: (error) => setSaveError(`保存失败：${kbErrorMessage(error)}`),
  });

  const handleSave = async () => {
    setSaved(false);
    setSaveError("");
    if (!name.trim()) {
      setSaveError("保存失败：知识库名称不能为空");
      return;
    }
    if (chunkOverlap >= chunkSize) {
      setSaveError("保存失败：分块重叠必须小于分块大小");
      return;
    }
    try { await saveMutation.mutateAsync(); } catch { /* Mutation callback reports the error. */ }
  };

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      {/* Basic */}
      <div className="surface data-card rounded-2xl">
        <h3 className="mb-4 text-sm font-semibold text-slate-900">基本信息</h3>
        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">名称</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
            />
          </div>
          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">描述</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={2}
              className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
            />
          </div>
          <div className="flex items-center gap-3">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={status === 1}
                onChange={(e) => setStatus(e.target.checked ? 1 : 0)}
                className="rounded"
              />
              <span className="text-sm text-slate-700">启用知识库</span>
            </label>
            <label className="flex items-center gap-2 cursor-pointer ml-4">
              <input
                type="checkbox"
                checked={mcpEnabled === 1}
                onChange={(e) => setMcpEnabled(e.target.checked ? 1 : 0)}
                className="rounded"
              />
              <span className="text-sm text-slate-700">MCP 暴露</span>
            </label>
          </div>
          <p className="text-xs text-slate-400">
            关闭 MCP 暴露后，该知识库不会出现在 MCP 工具的列表中，也不会被全局搜索命中。仍可通过显式指定 kb_id 访问。
          </p>
        </div>
      </div>

      {/* Embedding config */}
      <div className="surface data-card rounded-2xl">
        <h3 className="mb-4 text-sm font-semibold text-slate-900">Embedding 配置</h3>
        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">Embedding 模型</label>
            <input
              type="text"
              value={embeddingModel}
              onChange={(e) => setEmbeddingModel(e.target.value)}
              placeholder="text-embedding-3-small"
              className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
            />
            <p className="mt-1 text-xs text-slate-400">
              支持的模型取决于渠道，常见：text-embedding-3-small / text-embedding-3-large / text-embedding-ada-002
            </p>
          </div>

          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">绑定渠道（可选）</label>
            <div className="relative">
              <button
                type="button"
                onClick={() => setShowChannelPicker(!showChannelPicker)}
                className="flex w-full items-center justify-between rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
              >
                <span className={selectedEmbeddingChannel ? "text-slate-900" : "text-slate-400"}>
                  {selectedEmbeddingChannel
                    ? `${selectedEmbeddingChannel.name} (${selectedEmbeddingChannel.type})`
                    : "自动选择（默认）"}
                </span>
                <ChevronDown size={15} className={`shrink-0 text-slate-400 transition-transform ${showChannelPicker ? "rotate-180" : ""}`} />
              </button>

              {showChannelPicker && (
                <>
                  <div className="fixed inset-0 z-40" onClick={() => setShowChannelPicker(false)} />
                  <div className="absolute left-0 top-full z-50 mt-1.5 w-full rounded-2xl border border-slate-200 bg-white p-2 shadow-xl max-h-[280px] overflow-auto">
                    <button
                      type="button"
                      onClick={() => {
                        setEmbeddingChannelId("");
                        setShowChannelPicker(false);
                      }}
                      className={`flex w-full items-center justify-between rounded-xl px-3 py-2.5 text-sm transition-all ${
                        embeddingChannelId === ""
                          ? "bg-blue-50 text-blue-600 font-semibold"
                          : "text-slate-700 hover:bg-slate-50"
                      }`}
                    >
                      <span>自动选择（默认）</span>
                      {embeddingChannelId === "" && <Check size={14} className="shrink-0" />}
                    </button>
                    {activeChannels.map((c) => (
                      <button
                        key={c.id}
                        type="button"
                        onClick={() => {
                          setEmbeddingChannelId(c.id);
                          setShowChannelPicker(false);
                        }}
                        className={`flex w-full items-center justify-between rounded-xl px-3 py-2.5 text-sm transition-all ${
                          embeddingChannelId === c.id
                            ? "bg-blue-50 text-blue-600 font-semibold"
                            : "text-slate-700 hover:bg-slate-50"
                        }`}
                      >
                        <div className="flex items-center gap-2 min-w-0">
                          <span className="truncate">{c.name}</span>
                          <span className="rounded-full bg-slate-100 px-1.5 py-0.5 text-[10px] text-slate-500 shrink-0">
                            {c.type}
                          </span>
                        </div>
                        {embeddingChannelId === c.id && <Check size={14} className="shrink-0" />}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
            <p className="mt-1 text-xs text-slate-400">
              指定后，embedding 请求会优先使用该渠道。不指定则自动调度。
            </p>
            {channelsQuery.error ? <p className="mt-1 text-xs text-red-500">渠道列表读取失败：{kbErrorMessage(channelsQuery.error)}</p> : null}
          </div>
        </div>
      </div>

      {/* Chunking & Filtering config */}
      <div className="surface data-card rounded-2xl">
        <h3 className="mb-4 text-sm font-semibold text-slate-900">分块与过滤</h3>
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1 block text-sm font-medium text-slate-700">分块大小 (tokens)</label>
              <input
                type="number"
                value={chunkSize}
                onChange={(e) => setChunkSize(Number(e.target.value) || 512)}
                min={50}
                max={2000}
                step={50}
                className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
              />
              <p className="mt-1 text-xs text-slate-400">默认 512，越大上下文越完整但消耗更多 token</p>
            </div>
            <div>
              <label className="mb-1 block text-sm font-medium text-slate-700">分块重叠 (tokens)</label>
              <input
                type="number"
                value={chunkOverlap}
                onChange={(e) => setChunkOverlap(Number(e.target.value) || 64)}
                min={0}
                max={500}
                step={16}
                className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
              />
              <p className="mt-1 text-xs text-slate-400">默认 64，保持上下文连续性</p>
            </div>
            <div>
              <label className="mb-1 block text-sm font-medium text-slate-700">Embedding 批量大小</label>
              <input
                type="number"
                value={embeddingBatchSize}
                onChange={(e) => setEmbeddingBatchSize(Math.max(1, Number(e.target.value) || 32))}
                min={1}
                max={256}
                step={1}
                className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
              />
              <p className="mt-1 text-xs text-slate-400">每次发送给 embedding 渠道的文本数量，默认 32</p>
            </div>
          </div>
          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">排除目录（逗号分隔）</label>
            <input
              type="text"
              value={excludedDirs}
              onChange={(e) => setExcludedDirs(e.target.value)}
              placeholder="tests, examples, vendor"
              className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
            />
            <p className="mt-1 text-xs text-slate-400">导入 Git/本地目录时跳过这些目录（默认排除 .git, node_modules 等）</p>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1 block text-sm font-medium text-slate-700">排除文件（逗号分隔）</label>
              <input
                type="text"
                value={excludedFiles}
                onChange={(e) => setExcludedFiles(e.target.value)}
                placeholder="*.lock, *.min.js"
                className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
              />
            </div>
            <div>
              <label className="mb-1 block text-sm font-medium text-slate-700">包含文件类型（逗号分隔，空=全部）</label>
              <input
                type="text"
                value={includedFiles}
                onChange={(e) => setIncludedFiles(e.target.value)}
                placeholder="md, rs, ts, py"
                className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
              />
            </div>
          </div>
        </div>
      </div>

      {/* Stats */}
      <div className="surface data-card rounded-2xl">
        <h3 className="mb-4 text-sm font-semibold text-slate-900">统计</h3>
        <div className="grid grid-cols-3 gap-4">
          <div className="rounded-xl bg-slate-50 p-3 text-center">
            <div className="text-2xl font-bold text-slate-900">{kb.doc_count}</div>
            <div className="text-xs text-slate-500">文档数</div>
          </div>
          <div className="rounded-xl bg-slate-50 p-3 text-center">
            <div className="text-2xl font-bold text-slate-900">{kb.chunk_count}</div>
            <div className="text-xs text-slate-500">切片数</div>
          </div>
          <div className="rounded-xl bg-slate-50 p-3 text-center">
            <div className="text-2xl font-bold text-slate-900">{kb.total_tokens}</div>
            <div className="text-xs text-slate-500">总 Tokens</div>
          </div>
        </div>
      </div>

      {/* Save */}
      <div className="surface data-card rounded-2xl flex flex-wrap items-center justify-end gap-3">
        {saveError ? <div className="kb-notice kb-notice-warning mr-auto" role="alert">{saveError}</div> : null}
        <button
          type="button"
          onClick={() => void handleSave()}
          disabled={saveMutation.isPending}
          className="action-primary disabled:opacity-50"
        >
          {saveMutation.isPending ? <Loader2 className="h-4 w-4 animate-spin" /> : <SettingsIcon size={16} />}
          保存设置
        </button>
        {saved && (
          <span className="flex items-center gap-1 text-sm text-emerald-600">
            <CheckCircle2 size={16} /> 已保存
          </span>
        )}
      </div>
    </div>
  );
}

// ─── MCP Tab (per-KB) ───────────────────────────────────────────────────
