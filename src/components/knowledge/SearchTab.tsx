import { useEffect, useState } from "react";
import { Loader2, Search, Sparkles } from "lucide-react";
import { kbApi } from "../../lib/api";
import type { KbSearchResult, KbTag, KnowledgeBase } from "../../types";

export function SearchTab({ kb }: { kb: KnowledgeBase }) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<KbSearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [searched, setSearched] = useState(false);
  const [tags, setTags] = useState<KbTag[]>([]);
  const [tagsLoading, setTagsLoading] = useState(false);

  // Load tags for preset search terms
  useEffect(() => {
    if (kb.chunk_count === 0) return;
    let active = true;
    setTagsLoading(true);
    kbApi.getTags(kb.id, 8)
      .then((data) => { if (active) setTags(data); })
      .catch(() => {})
      .finally(() => { if (active) setTagsLoading(false); });
    return () => { active = false; };
  }, [kb.id, kb.chunk_count]);

  const handleSearch = async (searchQuery?: string) => {
    const q = (searchQuery ?? query).trim();
    if (!q) return;
    if (searchQuery) setQuery(searchQuery);
    setSearching(true);
    setSearched(true);
    try {
      const data = await kbApi.search({ query: q, kb_id: kb.id, top_k: 10 });
      setResults(data);
    } catch (e) {
      console.error(e);
    } finally {
      setSearching(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && !e.nativeEvent.isComposing && handleSearch()}
          placeholder="输入搜索内容..."
          className="flex-1 rounded-xl border border-slate-200 bg-white px-4 py-2.5 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
        />
        <button
          onClick={() => handleSearch()}
          disabled={searching || !query.trim()}
          className="action-primary disabled:opacity-50"
        >
          {searching ? <Loader2 className="h-4 w-4 animate-spin" /> : <Search size={16} />}
          搜索
        </button>
      </div>

      {/* Preset search terms */}
      {(tagsLoading || tags.length > 0) && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="flex items-center gap-1 text-[11px] font-medium text-slate-400">
            <Sparkles size={12} />
            快速检索
          </span>
          {tagsLoading ? (
            <>
              {[...Array(5)].map((_, i) => (
                <div key={i} className="h-6 w-16 animate-pulse rounded-full bg-slate-100" />
              ))}
            </>
          ) : (
            tags.map((tag) => (
              <button
                key={tag.word}
                onClick={() => setQuery(tag.word)}
                className="kb-suggestion-chip"
              >
                {tag.word}
              </button>
            ))
          )}
        </div>
      )}

      {searched && !searching && results.length === 0 && (
        <div className="surface empty-state rounded-2xl">
          <Search className="h-8 w-8 text-slate-300" />
          <p className="text-sm text-slate-500">未找到相关内容</p>
        </div>
      )}

      {results.length > 0 && (
        <div className="space-y-3">
          {results.map((r, i) => (
            <div key={r.chunk_id} className="surface rounded-xl p-4">
              <div className="mb-2 flex items-center gap-2">
                <span className="rounded bg-blue-50 px-2 py-0.5 text-[10px] font-medium text-blue-600">
                  #{i + 1}
                </span>
                <span className="text-xs font-medium text-slate-700">{r.filename}</span>
                <span className="text-xs text-slate-400">
                  相似度: {(r.score * 100).toFixed(1)}%
                </span>
              </div>
              <p className="text-sm text-slate-600 whitespace-pre-wrap line-clamp-6">
                {r.content}
              </p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

