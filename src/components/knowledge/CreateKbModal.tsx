import { useState } from "react";
import { kbApi } from "../../lib/api";
import type { KnowledgeBase } from "../../types";
import { kbErrorMessage } from "./helpers";

export function CreateKbModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (created: KnowledgeBase) => void | Promise<void>;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [embeddingModel, setEmbeddingModel] = useState("text-embedding-3-small");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCreate = async () => {
    if (!name.trim()) {
      setError("请输入知识库名称");
      return;
    }
    setCreating(true);
    setError(null);
    try {
      const created = await kbApi.create({
        name: name.trim(),
        description: description.trim() || undefined,
        embedding_model: embeddingModel || undefined,
      });
      await onCreated(created);
    } catch (e) {
      setError(kbErrorMessage(e));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30" onClick={onClose}>
      <div
        className="w-full max-w-md rounded-2xl bg-white p-6 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-lg font-semibold text-slate-900">新建知识库</h3>

        <div className="mt-4 space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">名称</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="例如：项目文档库"
              className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
            />
          </div>

          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">描述（可选）</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="知识库用途描述..."
              rows={2}
              className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
            />
          </div>

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
              复用已有渠道的 Embedding 模型，确保渠道支持该模型
            </p>
          </div>

          {error && (
            <div className="rounded-lg bg-red-50 p-3 text-sm text-red-600">{error}</div>
          )}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded-xl px-4 py-2 text-sm text-slate-500 hover:bg-slate-100"
          >
            取消
          </button>
          <button
            onClick={handleCreate}
            disabled={creating}
            className="action-primary disabled:opacity-50"
          >
            {creating ? "创建中..." : "创建"}
          </button>
        </div>
      </div>
    </div>
  );
}
