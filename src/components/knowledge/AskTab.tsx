import { useEffect, useState } from "react";
import { Check, ChevronDown, ChevronRight, ChevronUp, Loader2, MessageCircle, Sliders, Sparkles } from "lucide-react";
import { channelApi, kbApi } from "../../lib/api";
import type { Channel, ConversationMessage, KbRagAnswer, KbRetrievalDetail, KnowledgeBase } from "../../types";

export function AskTab({ kb }: { kb: KnowledgeBase }) {
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<KbRagAnswer | null>(null);
  const [asking, setAsking] = useState(false);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [selectedChannelId, setSelectedChannelId] = useState<string>("");
  const [selectedModel, setSelectedModel] = useState<string>("");
  const [showChannelPicker, setShowChannelPicker] = useState(false);
  const [showModelPicker, setShowModelPicker] = useState(false);
  const [conversation, setConversation] = useState<Array<{ role: "user" | "assistant"; content: string; sources?: KbRagAnswer["sources"]; retrievalDetails?: KbRetrievalDetail[] | null }>>([]);
  const [deepResearch, setDeepResearch] = useState(false);
  const [showSearchConfig, setShowSearchConfig] = useState(false);
  const [searchMode, setSearchMode] = useState<"hybrid" | "vector" | "keyword">("hybrid");
  const [vectorWeight, setVectorWeight] = useState(0.7);
  const [keywordWeight, setKeywordWeight] = useState(0.3);
  const [topK, setTopK] = useState(5);
  const [showRetrievalDetails, setShowRetrievalDetails] = useState<number | null>(null);

  // Persistence key for this KB's ask preferences
  const storageKey = `kb_ask_prefs_${kb.id}`;

  useEffect(() => {
    channelApi.getAll().then((chs) => {
      const active = chs.filter((c) => c.status === 1);
      setChannels(active);

      // Load saved preferences from localStorage
      try {
        const saved = localStorage.getItem(storageKey);
        if (saved) {
          const prefs = JSON.parse(saved);
          // Validate that saved channel still exists and is active
          const savedCh = active.find(c => c.id === prefs.channelId);
          if (savedCh) {
            setSelectedChannelId(savedCh.id);
            // Validate saved model exists in that channel
            if (prefs.model && savedCh.models.includes(prefs.model)) {
              setSelectedModel(prefs.model);
            } else {
              setSelectedModel(savedCh.models[0] || "");
            }
            return;
          }
        }
      } catch {}

      // Fallback: auto-select first channel with models
      const first = active.find((c) => c.models.length > 0);
      if (first) {
        setSelectedChannelId(first.id);
        setSelectedModel(first.models[0]);
      }
    }).catch(() => setChannels([]));
  }, [storageKey]);

  // Persist preferences when they change
  useEffect(() => {
    if (selectedChannelId && selectedModel) {
      localStorage.setItem(storageKey, JSON.stringify({
        channelId: selectedChannelId,
        model: selectedModel,
      }));
    }
  }, [storageKey, selectedChannelId, selectedModel]);

  // Models from selected channel
  const selectedChannel = channels.find((c) => c.id === selectedChannelId);
  const channelModels = selectedChannel?.models ?? [];

  const handleSelectChannel = (chId: string) => {
    setSelectedChannelId(chId);
    const ch = channels.find((c) => c.id === chId);
    if (ch && ch.models.length > 0) {
      setSelectedModel(ch.models[0]);
    } else {
      setSelectedModel("");
    }
    setShowChannelPicker(false);
  };

  const handleSelectModel = (model: string) => {
    setSelectedModel(model);
    setShowModelPicker(false);
  };

  const handleAsk = async () => {
    if (!question.trim()) return;
    setAsking(true);
    const userMsg = question;
    setQuestion("");
    setConversation((prev) => [...prev, { role: "user", content: userMsg }]);
    try {
      // Build history from current conversation (last 20 messages)
      const history: ConversationMessage[] = conversation.slice(-20).map((m) => ({
        role: m.role,
        content: m.content,
      }));

      const result = await kbApi.ask({
        question: userMsg,
        kb_id: kb.id,
        top_k: topK,
        model: selectedModel || undefined,
        history,
        deep_research: deepResearch,
        max_rounds: 5,
        vector_weight: searchMode === "hybrid" ? vectorWeight : undefined,
        keyword_weight: searchMode === "hybrid" ? keywordWeight : undefined,
        search_mode: searchMode,
      });
      setAnswer(result);
      setConversation((prev) => [
        ...prev,
        { role: "assistant", content: result.answer, sources: result.sources, retrievalDetails: result.retrieval_details },
      ]);
    } catch (e) {
      const errMsg = `请求失败: ${e}`;
      setAnswer({ answer: errMsg, sources: [], usage: null, retrieval_details: null });
      setConversation((prev) => [...prev, { role: "assistant", content: errMsg }]);
    } finally {
      setAsking(false);
    }
  };

  return (
    <div className="flex flex-col h-[calc(100vh-300px)] min-h-[360px]">
      {/* Model selector bar — top fixed */}
      <div className="flex items-center gap-3 border-b border-border bg-background/60 rounded-t-2xl px-4 py-3 shrink-0">
          {/* Channel selector */}
          <div className="relative">
            <button
              type="button"
              onClick={() => { setShowChannelPicker(!showChannelPicker); setShowModelPicker(false); }}
              className="flex items-center gap-2 rounded-xl border border-border bg-white px-3 py-2 text-xs font-medium transition-all hover:border-primary/40 hover:shadow-sm"
            >
              <span className="text-muted-foreground">渠道</span>
              <span className={selectedChannel ? "text-foreground truncate max-w-[120px]" : "text-muted-foreground"}>
                {selectedChannel?.name ?? "选择渠道"}
              </span>
              <ChevronDown size={13} className={`shrink-0 text-muted-foreground transition-transform ${showChannelPicker ? "rotate-180" : ""}`} />
            </button>

            {showChannelPicker && (
              <>
                <div className="fixed inset-0 z-40" onClick={() => setShowChannelPicker(false)} />
                <div className="absolute left-0 top-full z-50 mt-1.5 w-56 rounded-2xl border border-border bg-white p-2 shadow-xl max-h-[280px] overflow-auto">
                  <div className="px-2 py-1.5 text-[11px] font-semibold text-muted-foreground/70 uppercase tracking-wide">活跃渠道</div>
                  {channels.length === 0 ? (
                    <div className="px-3 py-2 text-xs text-muted-foreground">暂无可用渠道</div>
                  ) : channels.map((ch) => (
                    <button
                      key={ch.id}
                      type="button"
                      onClick={() => handleSelectChannel(ch.id)}
                      className={`flex w-full items-center justify-between rounded-xl px-3 py-2.5 text-sm transition-all ${
                        selectedChannelId === ch.id
                          ? "bg-primary/8 text-primary font-semibold"
                          : "text-foreground hover:bg-muted/60"
                      }`}
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <span className="truncate">{ch.name}</span>
                        <span className="rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground shrink-0">
                          {ch.type}
                        </span>
                      </div>
                      {selectedChannelId === ch.id && <Check size={14} className="shrink-0" />}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>

          {/* Arrow */}
          <ChevronRight size={14} className="shrink-0 text-muted-foreground/40" />

          {/* Model selector */}
          <div className="relative">
            <button
              type="button"
              onClick={() => { setShowModelPicker(!showModelPicker); setShowChannelPicker(false); }}
              disabled={!selectedChannelId}
              className="flex items-center gap-2 rounded-xl border border-border bg-white px-3 py-2 text-xs font-medium transition-all hover:border-primary/40 hover:shadow-sm disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <span className="text-muted-foreground">模型</span>
              <span className={selectedModel ? "text-foreground truncate max-w-[160px]" : "text-muted-foreground"}>
                {selectedModel || "选择模型"}
              </span>
              <ChevronDown size={13} className={`shrink-0 text-muted-foreground transition-transform ${showModelPicker ? "rotate-180" : ""}`} />
            </button>

            {showModelPicker && selectedChannelId && (
              <>
                <div className="fixed inset-0 z-40" onClick={() => setShowModelPicker(false)} />
                <div className="absolute left-0 top-full z-50 mt-1.5 w-56 rounded-2xl border border-border bg-white p-2 shadow-xl max-h-[280px] overflow-auto">
                  <div className="px-2 py-1.5 text-[11px] font-semibold text-muted-foreground/70 uppercase tracking-wide">
                    {selectedChannel?.name} 模型
                  </div>
                  {channelModels.length === 0 ? (
                    <div className="px-3 py-2 text-xs text-muted-foreground">该渠道未配置模型</div>
                  ) : channelModels.map((m) => (
                    <button
                      key={m}
                      type="button"
                      onClick={() => handleSelectModel(m)}
                      className={`flex w-full items-center justify-between rounded-xl px-3 py-2.5 text-sm font-mono transition-all ${
                        selectedModel === m
                          ? "bg-primary/8 text-primary font-semibold"
                          : "text-foreground hover:bg-muted/60"
                      }`}
                    >
                      <span className="truncate">{m}</span>
                      {selectedModel === m && <Check size={14} className="shrink-0" />}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>

          {/* Right side actions */}
          <div className="ml-auto flex items-center gap-2">
            {selectedModel && (
              <span className="hidden sm:inline-flex rounded-full bg-primary/8 px-2.5 py-1 text-[10px] font-medium text-primary">
                {selectedModel}
              </span>
            )}
            {/* Deep Research toggle */}
            <button
              onClick={() => setDeepResearch(!deepResearch)}
              className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-colors ${
                deepResearch
                  ? "bg-violet-50 text-violet-600 hover:bg-violet-100"
                  : "bg-slate-100 text-slate-400 hover:bg-slate-200"
              }`
              }
              title="Deep Research: 多轮迭代检索+综合分析"
            >
              <Sparkles size={12} />
              Deep Research
            </button>
            {/* Search config toggle */}
            <button
              onClick={() => setShowSearchConfig(!showSearchConfig)}
              className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-colors ${
                showSearchConfig
                  ? "bg-blue-50 text-blue-600 hover:bg-blue-100"
                  : "bg-slate-100 text-slate-400 hover:bg-slate-200"
              }`
              }
              title="检索配置: 模式/权重/top_k"
            >
              <Sliders size={12} />
              检索配置
            </button>
            {conversation.length > 0 && (
              <button
                onClick={() => { setConversation([]); setAnswer(null); }}
                className="rounded-lg px-2.5 py-1.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
              >
                清空对话
              </button>
            )}
          </div>
        </div>

        {/* Search config panel */}
        {showSearchConfig && (
          <div className="border-b border-border bg-slate-50/50 px-4 py-3 space-y-3 shrink-0">
            <div className="flex items-center gap-4 flex-wrap">
              {/* Search mode */}
              <div className="flex items-center gap-2">
                <span className="text-xs font-medium text-muted-foreground">检索模式</span>
                <div className="flex rounded-lg border border-border overflow-hidden">
                  {(["hybrid", "vector", "keyword"] as const).map((m) => (
                    <button
                      key={m}
                      onClick={() => setSearchMode(m)}
                      className={`px-2.5 py-1 text-xs transition-colors ${
                        searchMode === m
                          ? "bg-primary text-white"
                          : "bg-white text-muted-foreground hover:bg-slate-100"
                      }`}
                    >
                      {m === "hybrid" ? "混合" : m === "vector" ? "向量" : "关键词"}
                    </button>
                  ))}
                </div>
              </div>
              {/* Top K */}
              <div className="flex items-center gap-2">
                <span className="text-xs font-medium text-muted-foreground">Top K</span>
                <input
                  type="number"
                  min={1}
                  max={20}
                  value={topK}
                  onChange={(e) => setTopK(Math.max(1, Math.min(20, Number(e.target.value) || 5)))}
                  className="w-14 rounded-lg border border-border bg-white px-2 py-1 text-xs text-center outline-none focus:border-primary"
                />
              </div>
            </div>
            {/* Weights (only for hybrid) */}
            {searchMode === "hybrid" && (
              <div className="flex items-center gap-4 flex-wrap">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-muted-foreground">向量权重</span>
                  <input
                    type="range"
                    min={0}
                    max={1}
                    step={0.1}
                    value={vectorWeight}
                    onChange={(e) => {
                      const v = Number(e.target.value);
                      setVectorWeight(v);
                      setKeywordWeight(Math.round((1 - v) * 10) / 10);
                    }}
                    className="w-24 accent-primary"
                  />
                  <span className="text-xs text-muted-foreground w-8">{vectorWeight.toFixed(1)}</span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-muted-foreground">关键词权重</span>
                  <input
                    type="range"
                    min={0}
                    max={1}
                    step={0.1}
                    value={keywordWeight}
                    onChange={(e) => {
                      const v = Number(e.target.value);
                      setKeywordWeight(v);
                      setVectorWeight(Math.round((1 - v) * 10) / 10);
                    }}
                    className="w-24 accent-primary"
                  />
                  <span className="text-xs text-muted-foreground w-8">{keywordWeight.toFixed(1)}</span>
                </div>
              </div>
            )}
          </div>
        )}

        {/* Conversation area — flexible middle, scrollable */}
        <div className="flex-1 min-h-0 overflow-y-auto px-4 py-4 space-y-4">
          {conversation.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
              <MessageCircle className="h-10 w-10 text-muted-foreground/30" />
              <p className="mt-3 text-sm">向知识库提问，AI 将基于检索到的内容回答</p>
              <p className="mt-1 text-xs text-muted-foreground/70">
                {kb.doc_count} 文档 · {kb.chunk_count} 切片可供检索
              </p>
            </div>
          ) : (
            conversation.map((msg, i) => (
              <div key={i} className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}>
                <div
                  className={`max-w-[80%] rounded-2xl px-4 py-3 text-sm ${
                    msg.role === "user"
                      ? "bg-primary text-white"
                      : "bg-muted/50 text-foreground border border-border"
                  }`}
                >
                  <p className="whitespace-pre-wrap">{msg.content}</p>
                  {msg.sources && msg.sources.length > 0 && (
                    <div className="mt-3 space-y-1.5 border-t border-border/40 pt-3">
                      <div className="text-[10px] font-medium text-muted-foreground uppercase tracking-wide">引用来源</div>
                      {msg.sources.map((s, si) => (
                        <div key={si} className="rounded-lg bg-white/80 p-2 text-xs">
                          <div className="flex items-center justify-between">
                            <span className="font-medium text-foreground">{s.filename}</span>
                            <span className="text-muted-foreground">{(s.score * 100).toFixed(1)}%</span>
                          </div>
                          <p className="mt-0.5 text-muted-foreground line-clamp-2">{s.snippet}</p>
                        </div>
                      ))}
                    </div>
                  )}
                  {msg.retrievalDetails && msg.retrievalDetails.length > 0 && (
                    <div className="mt-2 border-t border-border/40 pt-2">
                      <button
                        onClick={() => setShowRetrievalDetails(showRetrievalDetails === i ? null : i)}
                        className="flex items-center gap-1 text-[10px] font-medium text-muted-foreground hover:text-foreground transition-colors"
                      >
                        {showRetrievalDetails === i ? <ChevronUp size={10} /> : <ChevronDown size={10} />}
                        检索详情 ({msg.retrievalDetails.length})
                      </button>
                      {showRetrievalDetails === i && (
                        <div className="mt-1.5 space-y-1">
                          {msg.retrievalDetails.map((rd, rdi) => (
                            <div key={rdi} className="rounded-lg bg-white/60 p-2 text-xs border border-border/40">
                              <div className="flex items-center justify-between gap-2">
                                <div className="flex items-center gap-1.5 min-w-0">
                                  <span className="font-medium text-foreground truncate">{rd.filename}</span>
                                  {rd.symbol_name && (
                                    <span className="shrink-0 rounded bg-primary/10 px-1 py-0.5 text-[9px] text-primary">
                                      {rd.symbol_name}
                                    </span>
                                  )}
                                </div>
                                <span className="shrink-0 text-muted-foreground">{(rd.score * 100).toFixed(1)}%</span>
                              </div>
                              <div className="mt-1 flex items-center gap-3 text-[9px] text-muted-foreground">
                                {rd.vector_score != null && (
                                  <span className="text-blue-500">向量: {(rd.vector_score * 100).toFixed(1)}%</span>
                                )}
                                {rd.keyword_score != null && (
                                  <span className="text-green-500">关键词: {(rd.keyword_score * 100).toFixed(1)}%</span>
                                )}
                              </div>
                              <p className="mt-0.5 text-muted-foreground line-clamp-2 text-[10px]">{rd.snippet}</p>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
            ))
          )}
          {asking && (
            <div className="flex justify-start">
              <div className="rounded-2xl bg-muted/50 border border-border px-4 py-3">
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  正在检索知识库并生成回答...
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Input bar — bottom fixed */}
        <div className="border-t border-border bg-background/40 rounded-b-2xl px-4 py-3 shrink-0">
          <div className="flex items-end gap-2">
            <textarea
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  handleAsk();
                }
              }}
              placeholder="输入问题，Ctrl/Command+Enter 发送，Enter 换行..."
              rows={1}
              className="flex-1 resize-none rounded-2xl border border-border bg-white px-3.5 py-2.5 text-sm outline-none focus:border-primary focus:ring-2 focus:ring-primary/20 max-h-32"
              style={{ minHeight: "42px" }}
              disabled={asking}
            />
            <button
              onClick={handleAsk}
              disabled={asking || !question.trim()}
              className="action-primary disabled:opacity-50 shrink-0"
            >
              {asking ? <Loader2 className="h-4 w-4 animate-spin" /> : <MessageCircle size={16} />}
              发送
            </button>
          </div>
          {/* Token usage */}
          {answer?.usage && (
            <div className="mt-2 flex items-center gap-3 text-[10px] text-muted-foreground">
              <span>Prompt: {answer.usage.prompt_tokens}</span>
              <span>Completion: {answer.usage.completion_tokens}</span>
              <span>Total: {answer.usage.total_tokens}</span>
            </div>
          )}
        </div>
    </div>
  );
}

// ─── Settings Tab ───────────────────────────────────────────────────────

