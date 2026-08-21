import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  BookOpen,
  CheckCircle2,
  Copy,
  Database,
  FileText,
  GitBranch,
  Layers,
  Loader2,
  MessageCircle,
  Plus,
  Search,
  Server,
  Settings as SettingsIcon,
  Sparkles,
  Terminal,
  Trash2,
  Upload,
  Wifi,
} from "lucide-react";
import { serverApi, serviceApi } from "../../lib/api";
import { errorMessage, queryKeys } from "../../lib/query";
import type { ServiceStatus } from "../../types";

const TOOL_ICONS: Record<string, typeof Terminal> = {
  search_knowledge_base: Search,
  list_knowledge_bases: BookOpen,
  read_document: FileText,
  ask_knowledge_base: MessageCircle,
  get_knowledge_base_stats: Database,
  create_knowledge_base: Plus,
  update_knowledge_base: SettingsIcon,
  delete_knowledge_base: Trash2,
  upload_document: Upload,
  delete_document: Trash2,
  list_documents: Layers,
  build_index: Sparkles,
  import_source: GitBranch,
  list_wiki_projects: BookOpen,
  search_wiki: Search,
  get_wiki_page: FileText,
  get_wiki_graph: GitBranch,
  list_background_tasks: Layers,
  get_background_task: Loader2,
};

function serviceState(service: ServiceStatus) {
  if (!service.running) return { className: "is-stopped", label: "已停止" };
  if (service.health === "degraded") return { className: "is-degraded", label: "受限" };
  return { className: "is-running", label: "运行中" };
}

export function McpServiceView() {
  const servicesQuery = useQuery({
    queryKey: queryKeys.serviceStatuses,
    queryFn: serviceApi.getStatuses,
    refetchInterval: 5_000,
  });
  const serverStatusQuery = useQuery({
    queryKey: queryKeys.serverStatus,
    queryFn: serverApi.getStatus,
    refetchInterval: 5_000,
  });
  const services = servicesQuery.data ?? [];
  const loading = servicesQuery.isPending || serverStatusQuery.isPending;
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState("");
  const serverUrl = serverStatusQuery.data?.running
    ? serverStatusQuery.data.url
    : "http://127.0.0.1:8777";

  const mcpService = services.find((service) => service.id === "mcp");
  const kbService = services.find((service) => service.id === "knowledge");
  const wikiService = services.find((service) => service.id === "wiki");
  const mcpEndpoint = `${serverUrl}/mcp`;
  const sseEndpoint = `${serverUrl}/mcp/sse`;
  const tools = (mcpService?.stats?.tools as { name: string; label: string; desc: string }[]) || [];

  const handleCopy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopyError("");
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
      setCopyError("当前环境无法访问剪贴板，请手动复制地址。");
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="h-8 w-8 animate-spin text-slate-400" />
      </div>
    );
  }

  if (servicesQuery.error || serverStatusQuery.error) {
    const failure = servicesQuery.error ?? serverStatusQuery.error;
    return (
      <div className="surface rounded-lg px-6 py-12 text-center text-sm text-danger" role="alert">
        <p>{errorMessage(failure)}</p>
        <button type="button" className="action-secondary mt-4" onClick={() => { void servicesQuery.refetch(); void serverStatusQuery.refetch(); }}>
          重新读取 MCP 状态
        </button>
      </div>
    );
  }

  return (
    <div className="mcp-dashboard">
      <section className="panel mcp-status-panel">
        <div className="panel-header panel-header-compact">
          <div className="mcp-panel-heading">
            <span className="mcp-panel-icon"><Server size={16} /></span>
            <div>
              <h2>服务状态</h2>
              <p>知识库与 MCP 运行概况</p>
            </div>
          </div>
        </div>
        <div className="mcp-panel-body mcp-status-list">
          {kbService ? (
            <div className="mcp-status-item">
              <div className="mcp-status-item-header">
                <span>知识库服务</span>
                <span className={`mcp-runtime-state ${serviceState(kbService).className}`}>
                  <Wifi size={12} />{serviceState(kbService).label}
                </span>
              </div>
              <div className="mcp-status-metrics">
                知识库: {String(kbService.stats.knowledge_bases || 0)} · 文档: {String(kbService.stats.documents || 0)} · 切片: {String(kbService.stats.chunks || 0)}
              </div>
              {kbService.issues[0] ? <div className="mcp-status-issue">{kbService.issues[0].message}</div> : null}
            </div>
          ) : null}
          {wikiService ? (
            <div className="mcp-status-item">
              <div className="mcp-status-item-header">
                <span>Wiki 服务</span>
                <span className={`mcp-runtime-state ${serviceState(wikiService).className}`}>
                  <Wifi size={12} />{serviceState(wikiService).label}
                </span>
              </div>
              <div className="mcp-status-metrics">
                项目: {String(wikiService.stats.projects || 0)} · 页面: {String(wikiService.stats.pages || 0)} · 来源: {String(wikiService.stats.sources || 0)}
              </div>
              {wikiService.issues[0] ? <div className="mcp-status-issue">{wikiService.issues[0].message}</div> : null}
            </div>
          ) : null}
          {mcpService ? (
            <div className="mcp-status-item">
              <div className="mcp-status-item-header">
                <span>MCP 服务</span>
                <span className={`mcp-runtime-state ${serviceState(mcpService).className}`}>
                  <Wifi size={12} />{serviceState(mcpService).label}
                </span>
              </div>
              <div className="mcp-status-metrics">
                可用知识库: {String(mcpService.stats.available_knowledge_bases || 0)} · 可用 Wiki: {String(mcpService.stats.available_wikis || 0)} · 工具: {tools.length}
              </div>
              {mcpService.issues[0] ? <div className="mcp-status-issue">{mcpService.issues[0].message}</div> : null}
            </div>
          ) : null}
          {!kbService && !wikiService && !mcpService ? <div className="mcp-inline-empty">等待桌面服务状态</div> : null}
        </div>
      </section>

      <section className="panel mcp-endpoints-panel">
        <div className="panel-header panel-header-compact">
          <div className="mcp-panel-heading">
            <span className="mcp-panel-icon mcp-panel-icon-blue"><Terminal size={16} /></span>
            <div>
              <h2>MCP 端点</h2>
              <p>JSON-RPC 与事件流入口</p>
            </div>
          </div>
        </div>
        <div className="mcp-panel-body mcp-endpoint-list">
          <div className="mcp-endpoint-row">
            <label>JSON-RPC 端点 <span>POST</span></label>
            <div className="mcp-endpoint-control">
              <code>{mcpEndpoint}</code>
              <button
                type="button"
                aria-label="复制 JSON-RPC 端点"
                title="复制 JSON-RPC 端点"
                onClick={() => void handleCopy(mcpEndpoint)}
                className="mcp-copy-button"
              >
                {copied ? <CheckCircle2 size={14} /> : <Copy size={14} />}
              </button>
            </div>
          </div>
          <div className="mcp-endpoint-row">
            <label>SSE 端点 <span>GET</span></label>
            <div className="mcp-endpoint-control">
              <code>{sseEndpoint}</code>
              <button
                type="button"
                aria-label="复制 SSE 端点"
                title="复制 SSE 端点"
                onClick={() => void handleCopy(sseEndpoint)}
                className="mcp-copy-button"
              >
                {copied ? <CheckCircle2 size={14} /> : <Copy size={14} />}
              </button>
            </div>
          </div>
          <div className="kb-notice kb-notice-warning">
            MCP 端点需要带有 MCP 只读或读写权限的 CrowAPI 密钥。
          </div>
          {copyError ? <div className="kb-notice kb-notice-warning" role="status">{copyError}</div> : null}
        </div>
      </section>

      <section className="panel mcp-tools-panel">
        <div className="panel-header">
          <div className="mcp-panel-heading">
            <span className="mcp-panel-icon"><Layers size={16} /></span>
            <div>
              <h2>可用工具</h2>
              <p>可由 MCP 客户端发现并调用的知识库能力</p>
            </div>
          </div>
          <span className="mcp-count-badge">{tools.length} 个</span>
        </div>
        <div className="mcp-panel-body mcp-tool-grid">
          {tools.map((tool) => {
            const Icon = TOOL_ICONS[tool.name] || Terminal;
            return (
              <article key={tool.name} className="mcp-tool-card">
                <div className="mcp-tool-icon"><Icon size={14} /></div>
                <div className="mcp-tool-content">
                  <strong>{tool.label}</strong>
                  <code>{tool.name}</code>
                  <p>{tool.desc}</p>
                </div>
              </article>
            );
          })}
          {tools.length === 0 ? <div className="mcp-tools-empty">桌面服务启动后将在此显示可用工具</div> : null}
        </div>
      </section>

      <section className="panel mcp-example-panel">
        <div className="panel-header panel-header-compact">
          <div className="mcp-panel-heading">
            <span className="mcp-panel-icon mcp-panel-icon-blue"><Terminal size={16} /></span>
            <div>
              <h2>调用示例</h2>
              <p>获取当前 MCP 工具清单</p>
            </div>
          </div>
        </div>
        <div className="mcp-panel-body">
          <pre className="mcp-code-block"><code>{`curl -X POST ${mcpEndpoint} \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer $CROWAPI_API_KEY" \\
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": {}
  }'`}</code></pre>
        </div>
      </section>
    </div>
  );
}
