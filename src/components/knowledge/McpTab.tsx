import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { CheckCircle2, Copy, Layers, Server, Terminal, Wifi } from "lucide-react";
import { serverApi } from "../../lib/api";
import { queryKeys } from "../../lib/query";
import type { KnowledgeBase } from "../../types";

export function McpTab({ kb }: { kb: KnowledgeBase }) {
  const serverStatusQuery = useQuery({
    queryKey: queryKeys.serverStatus,
    queryFn: serverApi.getStatus,
  });
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState("");
  const baseUrl = serverStatusQuery.data?.running
    ? serverStatusQuery.data.url
    : "http://127.0.0.1:8777";
  const mcpEndpoint = `${baseUrl}/mcp`;

  const handleCopy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopyError("");
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
      setCopyError("当前环境无法访问剪贴板，请手动复制地址。");
    }
  };

  const mcpTools = [
    { name: "search_knowledge_base", desc: "语义检索知识库，返回匹配文本片段和相似度评分", required: ["query"] },
    { name: "list_knowledge_bases", desc: "列出所有已暴露的知识库（ID/名称/文档数）", required: [] },
    { name: "ask_knowledge_base", desc: "RAG 问答，基于检索内容生成回答并返回来源引用", required: ["question"] },
    { name: "read_document", desc: "读取指定文档的完整内容", required: ["kb_id", "doc_id"] },
    { name: "get_knowledge_base_stats", desc: "获取知识库统计信息（文档数/切片数/token数）", required: ["kb_id"] },
  ];

  return (
    <div className="mcp-kb-stack">
      {/* 接入说明 */}
      <section className="panel">
        <div className="panel-header panel-header-compact">
          <div className="mcp-panel-heading">
            <span className="mcp-panel-icon mcp-panel-icon-blue"><Terminal size={16} /></span>
            <div>
              <h2>MCP 对接</h2>
              <p>当前知识库的协议入口与暴露状态</p>
            </div>
          </div>
          {kb.mcp_enabled === 1 ? (
            <span className="mcp-runtime-state is-running"><Wifi size={12} />已暴露</span>
          ) : (
            <span className="mcp-runtime-state"><Wifi size={12} />未暴露</span>
          )}
        </div>

        <div className="mcp-panel-body mcp-endpoint-list">
          {/* 端点地址 */}
          <div className="mcp-endpoint-row">
            <label>MCP 端点 <span>JSON-RPC over HTTP</span></label>
            <div className="mcp-endpoint-control">
              <code>{mcpEndpoint}</code>
              <button
                type="button"
                aria-label="复制 MCP 端点"
                title="复制 MCP 端点"
                onClick={() => handleCopy(mcpEndpoint)}
                className="mcp-copy-button"
              >
                {copied ? <CheckCircle2 size={14} /> : <Copy size={14} />}
              </button>
            </div>
          </div>

          {/* 协议说明 */}
          <div className="kb-notice kb-notice-info">
            使用带有 MCP 只读或读写权限的密钥访问此端点。
          </div>
          {copyError ? <div className="kb-notice kb-notice-warning" role="status">{copyError}</div> : null}

          {/* 未暴露提示 */}
          {kb.mcp_enabled !== 1 && (
            <div className="kb-notice kb-notice-warning">
              该知识库尚未开启 MCP 暴露，请先在「设置」中启用。
            </div>
          )}
        </div>
      </section>

      {/* 可用工具列表 */}
      <section className="panel">
        <div className="panel-header">
          <div className="mcp-panel-heading">
            <span className="mcp-panel-icon"><Layers size={16} /></span>
            <div>
              <h2>可用 MCP 工具</h2>
              <p>当前知识库支持的检索、问答与读取能力</p>
            </div>
          </div>
          <span className="mcp-count-badge">{mcpTools.length} 个</span>
        </div>
        <div className="mcp-panel-body mcp-tool-grid mcp-tool-grid-compact">
          {mcpTools.map((tool) => (
            <article key={tool.name} className="mcp-tool-card mcp-tool-card-compact">
              <div className="mcp-tool-content">
                <code>{tool.name}</code>
                <p>{tool.desc}</p>
                {tool.required.length > 0 && (
                  <span className="mcp-tool-params">
                    必填: {tool.required.join(", ")}
                  </span>
                )}
              </div>
            </article>
          ))}
        </div>
      </section>

      {/* 调用示例 */}
      <section className="panel">
        <div className="panel-header panel-header-compact">
          <div className="mcp-panel-heading">
            <span className="mcp-panel-icon mcp-panel-icon-blue"><Server size={16} /></span>
            <div>
              <h2>调用示例</h2>
              <p>按 MCP JSON-RPC 2.0 规范发起请求</p>
            </div>
          </div>
        </div>

        <div className="mcp-panel-body mcp-example-list">
          <div className="mcp-example-item">
            <label>1. 列出可用知识库</label>
            <pre className="mcp-code-block"><code>{`curl -X POST ${mcpEndpoint} \\
	  -H "Content-Type: application/json" \\
	  -H "Authorization: Bearer $CROWAPI_API_KEY" \\
	  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {
      "name": "list_knowledge_bases",
      "arguments": {}
    }
  }'`}</code></pre>
          </div>

          <div className="mcp-example-item">
            <label>2. 语义检索</label>
            <pre className="mcp-code-block"><code>{`curl -X POST ${mcpEndpoint} \\
	  -H "Content-Type: application/json" \\
	  -H "Authorization: Bearer $CROWAPI_API_KEY" \\
	  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "search_knowledge_base",
      "arguments": {
        "query": "你的检索内容",
        "kb_id": "${kb.id}",
        "top_k": 5
      }
    }
  }'`}</code></pre>
          </div>

          <div className="mcp-example-item">
            <label>3. RAG 问答</label>
            <pre className="mcp-code-block"><code>{`curl -X POST ${mcpEndpoint} \\
	  -H "Content-Type: application/json" \\
	  -H "Authorization: Bearer $CROWAPI_API_KEY" \\
	  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "ask_knowledge_base",
      "arguments": {
        "question": "你的问题",
        "kb_id": "${kb.id}"
      }
    }
  }'`}</code></pre>
          </div>

          <div className="kb-notice kb-notice-neutral">
            读写工具还需要密钥包含 MCP 读写权限。
          </div>
        </div>
      </section>
    </div>
  );
}

// ─── Create KB Modal ────────────────────────────────────────────────────
