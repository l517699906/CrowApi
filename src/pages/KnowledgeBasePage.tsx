import { useCallback, useEffect, useMemo, useState } from "react";
import { useLocation } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ServiceSwitcher } from "../components/ServiceSwitcher";
import { DocumentsTab } from "../components/knowledge/DocumentsTab";
import { IndexTab } from "../components/knowledge/IndexTab";
import { AskTab } from "../components/knowledge/AskTab";
import { CreateKbModal } from "../components/knowledge/CreateKbModal";
import { McpServiceView } from "../components/knowledge/McpServiceView";
import { McpTab } from "../components/knowledge/McpTab";
import { SearchTab } from "../components/knowledge/SearchTab";
import { SettingsTab } from "../components/knowledge/SettingsTab";
import { SourcesTab } from "../components/knowledge/SourcesTab";
import { kbErrorMessage } from "../components/knowledge/helpers";
import { Modal, PageTitle } from "../components/ui";
import {
  kbApi,
} from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type {
  KnowledgeBase,
} from "../types";
import {
  BookOpen,
  Plus,
  Trash2,
  Search,
  MessageCircle,
  FileText,
  Loader2,
  Hash,
  ChevronRight,
  Settings as SettingsIcon,
  Terminal,
  GitBranch,
  Database,
  Tag,
  ArrowLeft,
} from "lucide-react";

type ServiceTab = "knowledge" | "mcp";
type KbTab = "documents" | "sources" | "search" | "ask" | "settings" | "index" | "mcp";


export function KnowledgeBasePage() {
  const location = useLocation();
  const serviceTab: ServiceTab = location.pathname.startsWith("/services/mcp") ? "mcp" : "knowledge";
  const isMcp = serviceTab === "mcp";

  return (
    <div className="page-enter kb-page">
      <PageTitle
        title={isMcp ? "MCP 服务" : "知识库"}
        meta={isMcp ? "本地 MCP 端点、工具与接入配置" : "本地文档向量化、语义检索与 RAG 问答服务"}
        action={<ServiceSwitcher />}
      />

      <div className="kb-service-content">
        {serviceTab === "knowledge" ? <KnowledgeBaseSection /> : <McpServiceView />}
      </div>
    </div>
  );
}


// ─── Knowledge Base Section ──────────────────────────────────────────────

function KnowledgeBaseSection() {
  const queryClient = useQueryClient();
  const kbsQuery = useQuery({
    queryKey: queryKeys.knowledgeBases,
    queryFn: kbApi.getAll,
    staleTime: 5_000,
  });
  const kbs = kbsQuery.data ?? [];
  const [selectedKbId, setSelectedKbId] = useState<string | null>(null);
  const [kbTab, setKbTab] = useState<KbTab>("documents");
  const [showCreate, setShowCreate] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deleteTargetId, setDeleteTargetId] = useState<string | null>(null);

  const selectedKb = useMemo(
    () => (selectedKbId ? kbs.find((kb) => kb.id === selectedKbId) ?? null : null),
    [kbs, selectedKbId],
  );
  const deleteTarget = useMemo(
    () => (deleteTargetId ? kbs.find((kb) => kb.id === deleteTargetId) ?? null : null),
    [deleteTargetId, kbs],
  );

  useEffect(() => {
    if (selectedKbId && !selectedKb) {
      setSelectedKbId(null);
      setKbTab("documents");
    }
  }, [selectedKb, selectedKbId]);

  const refreshKbs = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.knowledgeBases });
  }, [queryClient]);

  const updateMutation = useMutation({
    mutationFn: ({ id, input }: { id: string; input: Parameters<typeof kbApi.update>[1] }) => kbApi.update(id, input),
    onSuccess: (updated) => {
      queryClient.setQueryData<KnowledgeBase[]>(queryKeys.knowledgeBases, (current) => (
        (current ?? []).map((item) => item.id === updated.id ? updated : item)
      ));
      queryClient.setQueryData(queryKeys.knowledgeBase(updated.id), updated);
      refreshKbs();
    },
    onError: (mutationError) => setError(kbErrorMessage(mutationError)),
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => kbApi.delete(id),
    onSuccess: (_result, id) => {
      if (selectedKbId === id) setSelectedKbId(null);
      setDeleteTargetId(null);
      refreshKbs();
      void queryClient.removeQueries({ queryKey: queryKeys.knowledgeBase(id) });
    },
    onError: (mutationError) => setError(kbErrorMessage(mutationError)),
  });

  const handleSelectKb = (kb: KnowledgeBase) => {
    setSelectedKbId(kb.id);
    setKbTab("documents");
  };

  const handleDelete = async () => {
    if (!deleteTarget) return;
    try { await deleteMutation.mutateAsync(deleteTarget.id); } catch { /* Mutation callback reports the error. */ }
  };

  // Toggle KB status (enable/disable) from list view
  const handleToggleStatus = async (kb: KnowledgeBase, newStatus: number) => {
    try { await updateMutation.mutateAsync({ id: kb.id, input: { status: newStatus } }); } catch { /* Mutation callback reports the error. */ }
  };

  // Toggle MCP exposure from list view
  const handleToggleMcp = async (kb: KnowledgeBase, newMcp: number) => {
    try { await updateMutation.mutateAsync({ id: kb.id, input: { mcp_enabled: newMcp } }); } catch { /* Mutation callback reports the error. */ }
  };

  return (
    <>
      {(error || kbsQuery.error) && (
        <div className="rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-600" role="alert">
          {error ?? errorMessage(kbsQuery.error)}
          <button type="button" aria-label="关闭错误提示" onClick={() => setError(null)} className="ml-2 text-red-400 hover:text-red-600">✕</button>
          {kbsQuery.error ? <button type="button" className="ml-3 underline" onClick={() => void kbsQuery.refetch()}>重试</button> : null}
        </div>
      )}

      {selectedKb ? (
        <KbDetail
          kb={selectedKb}
          tab={kbTab}
          setTab={setKbTab}
          onBack={() => { setSelectedKbId(null); setKbTab("documents"); }}
          onRefresh={refreshKbs}
        />
      ) : (
        <KbList
          kbs={kbs}
          loading={kbsQuery.isPending}
          onSelect={handleSelectKb}
          onDelete={(kb) => setDeleteTargetId(kb.id)}
          onCreate={() => setShowCreate(true)}
          onToggleStatus={handleToggleStatus}
          onToggleMcp={handleToggleMcp}
        />
      )}

      {showCreate && (
        <CreateKbModal
          onClose={() => setShowCreate(false)}
          onCreated={async (created) => {
            setShowCreate(false);
            queryClient.setQueryData<KnowledgeBase[]>(queryKeys.knowledgeBases, (current) => (
              [...(current ?? []).filter((item) => item.id !== created.id), created]
            ));
            refreshKbs();
          }}
        />
      )}

      {deleteTarget && (
        <Modal
          title="删除知识库"
          description="删除后无法恢复，关联文档、切片与索引也会一并移除。"
          onClose={() => { if (!deleteMutation.isPending) setDeleteTargetId(null); }}
          footer={(
            <>
              <button type="button" className="action-secondary" disabled={deleteMutation.isPending} onClick={() => setDeleteTargetId(null)}>取消</button>
              <button type="button" className="button-danger" disabled={deleteMutation.isPending} onClick={() => void handleDelete()}>
                {deleteMutation.isPending ? <Loader2 className="animate-spin" size={15} /> : <Trash2 size={15} />}
                删除知识库
              </button>
            </>
          )}
        >
          <p className="text-sm text-muted">确定删除 <strong className="text-ink">{deleteTarget.name}</strong>？</p>
        </Modal>
      )}
    </>
  );
}

// ─── KB Tags Bar (high-frequency words) ─────────────────────────────

function KbTagsBar({ kbId, chunkCount }: { kbId: string; chunkCount: number }) {
  const tagsQuery = useQuery({
    queryKey: queryKeys.kbTags(kbId),
    queryFn: () => kbApi.getTags(kbId, 12),
    enabled: chunkCount > 0,
    staleTime: 30_000,
  });
  const tags = tagsQuery.data ?? [];
  const loading = tagsQuery.isPending;

  if (loading && tags.length === 0) {
    return (
      <div className="mt-2.5 flex items-center gap-1.5">
        {[...Array(5)].map((_, i) => (
          <div key={i} className="h-5 w-12 animate-pulse rounded-full bg-slate-100" />
        ))}
      </div>
    );
  }

  if (tags.length === 0) return null;

  // Color palette for tags - gradient blues/purples for visual appeal
  const tagColors = [
    "bg-blue-50 text-blue-600 border-blue-100",
    "bg-violet-50 text-violet-600 border-violet-100",
    "bg-emerald-50 text-emerald-600 border-emerald-100",
    "bg-amber-50 text-amber-600 border-amber-100",
    "bg-rose-50 text-rose-500 border-rose-100",
    "bg-cyan-50 text-cyan-600 border-cyan-100",
    "bg-indigo-50 text-indigo-600 border-indigo-100",
    "bg-teal-50 text-teal-600 border-teal-100",
  ];

  return (
    <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
      <Tag size={11} className="text-slate-400 shrink-0" />
      {tags.map((tag, i) => (
        <span
          key={tag.word}
          className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-medium ${tagColors[i % tagColors.length]}`}
        >
          {tag.word}
        </span>
      ))}
    </div>
  );
}

// ─── KB List ────────────────────────────────────────────────────────────

function KbList({
  kbs,
  loading,
  onSelect,
  onDelete,
  onCreate,
  onToggleStatus,
  onToggleMcp,
}: {
  kbs: KnowledgeBase[];
  loading: boolean;
  onSelect: (kb: KnowledgeBase) => void;
  onDelete: (kb: KnowledgeBase) => void;
  onCreate: () => void;
  onToggleStatus: (kb: KnowledgeBase, newStatus: number) => void;
  onToggleMcp: (kb: KnowledgeBase, newMcp: number) => void;
}) {
  if (loading && kbs.length === 0) {
    return (
      <div className="surface empty-state">
        <Loader2 className="h-8 w-8 animate-spin text-slate-400" />
      </div>
    );
  }

  if (kbs.length === 0) {
    return (
      <div className="surface empty-state">
        <BookOpen className="h-12 w-12 text-slate-300" />
        <p className="text-sm text-slate-500">还没有知识库</p>
        <button onClick={onCreate} className="action-primary mt-2">
          <Plus size={16} />
          新建知识库
        </button>
      </div>
    );
  }

  return (
    <>
      <div className="kb-list-toolbar">
        <div>
          <h2>知识库列表</h2>
          <p>{kbs.length} 个知识库 · 选择条目进入文档与索引管理</p>
        </div>
        <button onClick={onCreate} className="action-primary">
          <Plus size={16} />
          新建知识库
        </button>
      </div>
      <div className="kb-list">
        {kbs.map((kb) => (
          <article
            key={kb.id}
            className="surface group kb-list-card"
          >
            <div className="kb-list-card-layout">
              {/* Icon */}
              <div className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl ${kb.status === 1 ? "bg-blue-50" : "bg-slate-100"}`}>
                <BookOpen className={`h-5 w-5 ${kb.status === 1 ? "text-blue-600" : "text-slate-400"}`} />
              </div>

              {/* Main content - clickable */}
              <div
                className="min-w-0 flex-1 cursor-pointer"
                onClick={() => onSelect(kb)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    onSelect(kb);
                  }
                }}
                role="button"
                tabIndex={0}
                aria-label={`打开知识库 ${kb.name}`}
              >
                <div className="flex items-center gap-2">
                  <h3 className="text-base font-semibold text-slate-900">{kb.name}</h3>
                  {kb.status === 1 ? (
                    <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[10px] font-medium text-emerald-600">活跃</span>
                  ) : (
                    <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[10px] font-medium text-slate-500">已禁用</span>
                  )}
                </div>
                <p className="mt-0.5 text-xs text-slate-500 line-clamp-1">
                  {kb.description || "暂无描述"}
                </p>
                <div className="mt-2 flex items-center gap-4 text-xs text-slate-500">
                  <span className="flex items-center gap-1">
                    <FileText size={12} /> {kb.doc_count} 文档
                  </span>
                  <span className="flex items-center gap-1">
                    <Hash size={12} /> {kb.chunk_count} 切片
                  </span>
                  {kb.embedding_model && (
                    <span className="truncate" title={kb.embedding_model}>
                      {kb.embedding_model}
                    </span>
                  )}
                </div>
                {/* Tags */}
                <KbTagsBar kbId={kb.id} chunkCount={kb.chunk_count} />
              </div>

              {/* Right side: toggles + actions */}
              <div className="kb-list-card-actions">
                <div className="kb-list-card-controls">
                  {/* MCP toggle */}
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      onToggleMcp(kb, kb.mcp_enabled === 1 ? 0 : 1);
                    }}
                    className={`flex items-center gap-1.5 rounded-lg px-2 py-1 text-[10px] font-medium transition-colors ${
                      kb.mcp_enabled === 1
                        ? "bg-violet-50 text-violet-600 hover:bg-violet-100"
                        : "bg-slate-100 text-slate-400 hover:bg-slate-200"
                    }`}
                    title="MCP 暴露开关"
                    aria-label={`${kb.name} MCP 暴露${kb.mcp_enabled === 1 ? "已开启" : "已关闭"}`}
                    aria-pressed={kb.mcp_enabled === 1}
                  >
                    <Terminal size={11} />
                    MCP {kb.mcp_enabled === 1 ? "已暴露" : "未暴露"}
                  </button>

                  {/* Status toggle */}
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      onToggleStatus(kb, kb.status === 1 ? 0 : 1);
                    }}
                    className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors ${
                      kb.status === 1 ? "bg-emerald-500" : "bg-slate-300"
                    }`}
                    title="知识库开关"
                    aria-label={`${kb.name} ${kb.status === 1 ? "已启用" : "已停用"}`}
                    aria-pressed={kb.status === 1}
                  >
                    <span
                      className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform ${
                        kb.status === 1 ? "translate-x-4" : "translate-x-1"
                      }`}
                    />
                  </button>

                  {/* Delete */}
                  <button
                    type="button"
                    onClick={(e) => { e.stopPropagation(); onDelete(kb); }}
                    className="kb-delete-button rounded-lg p-1.5 text-slate-400 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-red-50 hover:text-red-500"
                    aria-label={`删除知识库 ${kb.name}`}
                    title="删除知识库"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
                <ChevronRight size={16} className="text-slate-300 group-hover:text-blue-500" />
              </div>
            </div>
          </article>
        ))}
      </div>
    </>
  );
}

// ─── KB Detail ───────────────────────────────────────────────────────────

function KbDetail({
  kb,
  tab,
  setTab,
  onBack,
  onRefresh,
}: {
  kb: KnowledgeBase;
  tab: KbTab;
  setTab: (t: KbTab) => void;
  onBack: () => void;
  onRefresh: () => void;
}) {
  const tabs: { key: KbTab; label: string; icon: typeof FileText }[] = [
    { key: "documents", label: "文档", icon: FileText },
    { key: "sources", label: "来源", icon: GitBranch },
    { key: "search", label: "检索", icon: Search },
    { key: "ask", label: "问答", icon: MessageCircle },
    { key: "index", label: "索引", icon: Database },
    { key: "settings", label: "设置", icon: SettingsIcon },
    { key: "mcp", label: "MCP", icon: Terminal },
  ];

  return (
    <div className="kb-detail">
      <div className="kb-detail-header">
        <div className="kb-detail-summary">
          <button
            type="button"
            onClick={onBack}
            className="kb-back-button"
            aria-label="返回知识库列表"
          >
            <ArrowLeft size={16} />
            返回
          </button>
          <div className="kb-detail-title">
            <h2>{kb.name}</h2>
            <span>{kb.doc_count} 文档 · {kb.chunk_count} 切片</span>
          </div>
        </div>

        <div className="kb-detail-tabs" role="tablist" aria-label={`${kb.name} 管理视图`}>
          {tabs.map(({ key, label, icon: Icon }) => (
            <button
              key={key}
              type="button"
              role="tab"
              id={`kb-tab-${key}`}
              aria-controls={`kb-panel-${key}`}
              aria-selected={tab === key}
              onClick={() => setTab(key)}
              className={tab === key ? "is-active" : ""}
            >
              <Icon size={15} />
              {label}
            </button>
          ))}
        </div>
      </div>

      <div id={`kb-panel-${tab}`} className="kb-detail-content" role="tabpanel" aria-labelledby={`kb-tab-${tab}`}>
        {tab === "documents" && <DocumentsTab kb={kb} onRefresh={onRefresh} />}
        {tab === "sources" && <SourcesTab kb={kb} onRefresh={onRefresh} />}
        {tab === "search" && <SearchTab kb={kb} />}
        {tab === "ask" && <AskTab kb={kb} />}
        {tab === "index" && <IndexTab kb={kb} />}
        {tab === "settings" && <SettingsTab kb={kb} onRefresh={onRefresh} />}
        {tab === "mcp" && <McpTab kb={kb} />}
      </div>
    </div>
  );
}

// ─── Sources Tab (Multi-source import) ────────────────────────────────







// ─── Helpers ────────────────────────────────────────────────────────────
