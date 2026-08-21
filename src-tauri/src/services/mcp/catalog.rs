use crate::core::access::ACCESS_SCOPE_MCP_WRITE;
use crate::server::auth::AuthenticatedPrincipal;

/// MCP server instructions — agent 首次连接时注入 system prompt
pub(crate) const MCP_INSTRUCTIONS: &str = r#"# CrowAPI RAG — 本地向量检索

RAG 已预建索引：文档已解析、分块、向量化并存入本地 SQLite + HNSW 索引。
所有检索都是本地操作，亚秒级响应。

## 工具使用优先级

1. **ask_knowledge_base** — 首选。直接提问，返回 AI 生成的回答 + 来源引用。
   适合：任何问题、概念理解、代码含义、流程梳理。

2. **search_knowledge_base** — 当需要看原始文本片段，或 ask_knowledge_base 回答不够时使用。
   返回匹配的 chunk 原文 + 相似度分数。

3. **list_knowledge_bases** — 首次使用时调用一次，获取可用 RAG ID。
   之后无需重复调用。

4. **其他工具** — 按需使用（上传文档、管理索引等）。

## 反模式

- ❌ 不要先 search 再自己总结 — 直接用 ask_knowledge_base，它内部已做 RAG
- ❌ 不要每次都调 list_knowledge_bases — 缓存第一次的结果
- ❌ 不要对同一问题反复 search 不同关键词 — 一次 ask_knowledge_base 通常足够

## 代码文件

RAG 中的代码文件按符号边界分块（函数/类/方法），每个 chunk 是完整符号。
chunk metadata 包含 symbol_name、symbol_kind、signature，可用于精确过滤。"#;

// ── MCP tool definitions ──────────────────────────────────────────

const MCP_WRITE_TOOLS: &[&str] = &[
    "cancel_background_task",
    "retry_background_task",
    "create_knowledge_base",
    "update_knowledge_base",
    "delete_knowledge_base",
    "upload_document",
    "delete_document",
    "build_index",
    "import_source",
    "save_wiki_page",
    "ingest_wiki_source",
    "create_wiki_project",
    "delete_wiki_project",
    "delete_wiki_page",
    "add_wiki_source",
    "delete_wiki_source",
];

pub(crate) fn mcp_tool_requires_write(tool_name: &str) -> bool {
    MCP_WRITE_TOOLS.contains(&tool_name)
}

fn tool_catalog() -> Vec<serde_json::Value> {
    vec![
        // ── Read-only tools (existing) ───────────────────────────
        serde_json::json!({
            "name": "search_knowledge_base",
            "description": "Search across a local knowledge base using hybrid (vector + keyword), vector-only, or keyword-only retrieval. Returns matching text chunks with similarity scores and per-component score breakdowns. Searches a specific KB if kb_id provided, otherwise searches all MCP-enabled KBs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language search query. Will be embedded and matched against document chunks."
                    },
                    "kb_id": {
                        "type": "string",
                        "description": "Specific knowledge base ID to search. If omitted, searches all MCP-enabled KBs."
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 5)",
                        "default": 5
                    },
                    "search_mode": {
                        "type": "string",
                        "enum": ["hybrid", "vector", "keyword"],
                        "description": "Retrieval mode: hybrid (vector+keyword, default), vector (semantic only), keyword (FTS5 only). CJK bigram tokenization is used for Chinese queries.",
                        "default": "hybrid"
                    },
                    "vector_weight": {
                        "type": "number",
                        "description": "Weight for vector similarity score in hybrid mode (0.0-1.0, default: 0.7). Only effective when search_mode=hybrid.",
                        "default": 0.7
                    },
                    "keyword_weight": {
                        "type": "number",
                        "description": "Weight for keyword (FTS5) score in hybrid mode (0.0-1.0, default: 0.3). Only effective when search_mode=hybrid.",
                        "default": 0.3
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "list_knowledge_bases",
            "description": "List all MCP-enabled knowledge bases. Returns KB ID, name, document count, chunk count, and description. Use this first to discover available knowledge bases before searching or asking questions.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        serde_json::json!({
            "name": "read_document",
            "description": "Read the full content of a specific document in a knowledge base. Use after search_knowledge_base to get the complete text of a matched chunk's source document.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Knowledge base ID" },
                    "doc_id": { "type": "string", "description": "Document ID (from search results)" }
                },
                "required": ["kb_id", "doc_id"]
            }
        }),
        serde_json::json!({
            "name": "ask_knowledge_base",
            "description": "Ask a question to the knowledge base and get an AI-generated answer based on retrieved context (RAG). Uses the configured LLM channel to generate a response grounded in the KB content. Returns the answer, source citations, and per-chunk retrieval score breakdowns (vector score, keyword score, symbol info).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The question to ask. The answer will be grounded in retrieved document chunks." },
                    "kb_id": { "type": "string", "description": "Knowledge base ID. If omitted, uses all MCP-enabled KBs." },
                    "top_k": { "type": "integer", "description": "Number of chunks to retrieve as context (default: 5)", "default": 5 },
                    "model": { "type": "string", "description": "LLM model to use for answer generation (default: uses channel default)" },
                    "search_mode": {
                        "type": "string",
                        "enum": ["hybrid", "vector", "keyword"],
                        "description": "Retrieval mode: hybrid (vector+keyword, default), vector (semantic only), keyword (FTS5 only). CJK bigram tokenization is used for Chinese queries.",
                        "default": "hybrid"
                    },
                    "vector_weight": {
                        "type": "number",
                        "description": "Weight for vector similarity in hybrid mode (0.0-1.0, default: 0.7). Only effective when search_mode=hybrid.",
                        "default": 0.7
                    },
                    "keyword_weight": {
                        "type": "number",
                        "description": "Weight for keyword (FTS5) in hybrid mode (0.0-1.0, default: 0.3). Only effective when search_mode=hybrid.",
                        "default": 0.3
                    }
                },
                "required": ["question"]
            }
        }),
        serde_json::json!({
            "name": "get_knowledge_base_stats",
            "description": "Get detailed statistics about a knowledge base: document count, ready documents, chunk count, total tokens, embedding model, and index status.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Knowledge base ID" }
                },
                "required": ["kb_id"]
            }
        }),
        // ── Write tools: Knowledge Base lifecycle ──────────────────
        serde_json::json!({
            "name": "create_knowledge_base",
            "description": "创建新 RAG 知识库。⚠️ 使用前请先调用 list_knowledge_bases 查看已有知识库，避免重复创建。仅当用户明确要求创建新库，或现有库都不适用时才创建。创建后可通过 upload_document 或 import_source 添加内容。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "RAG 名称（1-100字符）" },
                    "description": { "type": "string", "description": "RAG 用途描述（可选）" },
                    "embedding_model": { "type": "string", "description": "嵌入模型（默认: text-embedding-3-small）" },
                    "embedding_channel_id": { "type": "string", "description": "自定义嵌入渠道 ID（可选）" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "update_knowledge_base",
            "description": "Update knowledge base configuration: name, description, embedding model, chunk size, MCP enabled status, etc. Only provided fields will be updated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Knowledge base ID" },
                    "name": { "type": "string", "description": "New name" },
                    "description": { "type": "string", "description": "New description" },
                    "embedding_model": { "type": "string", "description": "New embedding model (changing this requires re-indexing all documents)" },
                    "embedding_channel_id": { "type": "string", "description": "New embedding channel ID" },
                    "mcp_enabled": { "type": "integer", "description": "1 to enable MCP access, 0 to disable" },
                    "chunk_size": { "type": "integer", "description": "Chunk size in tokens (default: 512)" },
                    "chunk_overlap": { "type": "integer", "description": "Chunk overlap in tokens (default: 64)" }
                },
                "required": ["kb_id"]
            }
        }),
        serde_json::json!({
            "name": "delete_knowledge_base",
            "description": "Permanently delete a knowledge base and all its documents, chunks, and index. This action cannot be undone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Knowledge base ID to delete" }
                },
                "required": ["kb_id"]
            }
        }),
        // ── Write tools: Document management ───────────────────────
        serde_json::json!({
            "name": "upload_document",
            "description": "上传文档到 RAG。⚠️ 如果未指定 kb_id，将返回已有知识库列表供选择——请先调用 list_knowledge_bases 让用户选择目标库，或确认创建新库。文档上传后会自动解析、分块、向量化并建立索引。支持格式: .txt .md .pdf .docx .doc .pptx .xlsx .csv .json .html .rs .py .js .ts .go .java .c .cpp .h .sh .yaml .yml .toml",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "目标 RAG ID。如果未提供，将返回已有知识库列表供用户选择" },
                    "filename": { "type": "string", "description": "文档文件名（含扩展名，如 'report.pdf'）" },
                    "content": { "type": "string", "description": "Base64 编码的文件内容" }
                },
                "required": ["filename", "content"]
            }
        }),
        serde_json::json!({
            "name": "delete_document",
            "description": "Delete a document from a knowledge base. This removes the document, its chunks, and its embeddings. The HNSW index will be rebuilt automatically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Knowledge base ID" },
                    "doc_id": { "type": "string", "description": "Document ID to delete" }
                },
                "required": ["kb_id", "doc_id"]
            }
        }),
        serde_json::json!({
            "name": "list_documents",
            "description": "List all documents in a knowledge base with their status (pending, processing, ready, failed), chunk count, and token count.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Knowledge base ID" }
                },
                "required": ["kb_id"]
            }
        }),
        // ── Write tools: Index management ──────────────────────────
        serde_json::json!({
            "name": "build_index",
            "description": "Build or rebuild the HNSW vector index for a knowledge base. This should be called after uploading multiple documents to optimize search performance. The build runs asynchronously.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Knowledge base ID" }
                },
                "required": ["kb_id"]
            }
        }),
        // ── Write tools: Source import ─────────────────────────────
        serde_json::json!({
            "name": "import_source",
            "description": "Import documents from an external source (Git repo, URL, or local directory) into a knowledge base. The import runs asynchronously — use list_documents or get_knowledge_base_stats to check progress.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "description": "Target knowledge base ID" },
                    "source_type": { "type": "string", "enum": ["git", "url", "local_dir"], "description": "Type of source to import from" },
                    "repo_url": { "type": "string", "description": "Git repository URL (for source_type=git)" },
                    "branch": { "type": "string", "description": "Git branch to clone (for source_type=git, default: main)" },
                    "token": { "type": "string", "description": "Git access token for private repos (for source_type=git)" },
                    "url": { "type": "string", "description": "URL to fetch content from (for source_type=url)" },
                    "dir_path": { "type": "string", "description": "Local directory path (for source_type=local_dir)" },
                    "excluded_dirs": { "type": "array", "items": { "type": "string" }, "description": "Directory names to exclude (e.g. ['node_modules', '.git'])" },
                    "included_files": { "type": "array", "items": { "type": "string" }, "description": "File extensions to include (e.g. ['.md', '.txt'])" },
                    "max_file_size": { "type": "integer", "description": "Max file size in bytes (default: 1MB)" }
                },
                "required": ["kb_id", "source_type"]
            }
        }),
        // ── Background task tools ───────────────────────────────────
        serde_json::json!({
            "name": "list_background_tasks",
            "description": "List background tasks for one MCP-enabled knowledge base or Wiki project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "domain": { "type": "string", "enum": ["knowledge", "wiki"] },
                    "resource_id": { "type": "string", "description": "Knowledge base ID or Wiki project ID" },
                    "status": { "type": "string", "enum": ["pending", "running", "succeeded", "failed", "cancelled", "interrupted"] },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 }
                },
                "required": ["domain", "resource_id"]
            }
        }),
        serde_json::json!({
            "name": "get_background_task",
            "description": "Get one background task, including progress, stage, retry lineage, and error details.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" }
                },
                "required": ["task_id"]
            }
        }),
        serde_json::json!({
            "name": "cancel_background_task",
            "description": "Request cancellation for a pending or running background task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" }
                },
                "required": ["task_id"]
            }
        }),
        serde_json::json!({
            "name": "retry_background_task",
            "description": "Create and immediately dispatch a retry for a failed, cancelled, or interrupted retryable task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" }
                },
                "required": ["task_id"]
            }
        }),
        // ── Wiki tools: Project management ──────────────────────────
        serde_json::json!({
            "name": "list_wiki_projects",
            "description": "列出所有 Wiki 项目。返回项目 ID、名称、页面数、源数、描述。Wiki 是结构化知识库，页面按 frontmatter 组织，支持标签、图谱、问答。",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        serde_json::json!({
            "name": "get_wiki_project",
            "description": "获取 Wiki 项目详情：统计信息、标签、页面概览。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" }
                },
                "required": ["project_id"]
            }
        }),
        // ── Wiki tools: Pages ───────────────────────────────────────
        serde_json::json!({
            "name": "list_wiki_pages",
            "description": "列出 Wiki 项目的所有页面，返回路径、标题、类型、标签。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" }
                },
                "required": ["project_id"]
            }
        }),
        serde_json::json!({
            "name": "get_wiki_page",
            "description": "读取 Wiki 页面的完整 Markdown 内容。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "path": { "type": "string", "description": "页面路径（如 'index.md' 或 'guides/setup.md'）" }
                },
                "required": ["project_id", "path"]
            }
        }),
        serde_json::json!({
            "name": "save_wiki_page",
            "description": "创建或更新 Wiki 页面。传入 Markdown 内容，自动提取 frontmatter 标签和 wikilinks。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "path": { "type": "string", "description": "页面路径（如 'guides/api.md'）" },
                    "content": { "type": "string", "description": "页面 Markdown 内容" }
                },
                "required": ["project_id", "path", "content"]
            }
        }),
        // ── Wiki tools: Search & Ask ───────────────────────────────
        serde_json::json!({
            "name": "search_wiki",
            "description": "搜索 Wiki 页面。按标题、路径模糊匹配，并搜索页面内容。返回匹配页面的标题、路径、摘要片段。适合结构化知识检索，比 RAG chunk 更精确。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "query": { "type": "string", "description": "搜索关键词" },
                    "top_k": { "type": "integer", "description": "最大返回结果数（默认 10）", "default": 10 }
                },
                "required": ["project_id", "query"]
            }
        }),
        serde_json::json!({
            "name": "ask_wiki",
            "description": "向 Wiki 提问，获取基于 Wiki 页面的 AI 回答。检索相关页面 → LLM 生成回答 → 返回回答 + 来源引用。与 RAG ask 类似，但基于完整 Wiki 页面而非 chunk。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "question": { "type": "string", "description": "问题" },
                    "top_k": { "type": "integer", "description": "检索页面数（默认 5）", "default": 5 },
                    "model": { "type": "string", "description": "LLM 模型（默认用项目配置）" }
                },
                "required": ["project_id", "question"]
            }
        }),
        // ── Wiki tools: Tags & Graph ───────────────────────────────
        serde_json::json!({
            "name": "get_wiki_tags",
            "description": "获取 Wiki 项目的标签列表（从页面 frontmatter 自动提取），按频率排序。可用于快速了解 Wiki 覆盖的主题。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "limit": { "type": "integer", "description": "返回标签数（默认 15）", "default": 15 }
                },
                "required": ["project_id"]
            }
        }),
        serde_json::json!({
            "name": "get_wiki_graph",
            "description": "获取 Wiki 知识图谱：页面（节点）和 wikilinks（边）。用于可视化知识关联。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" }
                },
                "required": ["project_id"]
            }
        }),
        // ── Wiki tools: Sources ─────────────────────────────────────
        serde_json::json!({
            "name": "list_wiki_sources",
            "description": "列出 Wiki 项目的源资料及其摄入状态。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" }
                },
                "required": ["project_id"]
            }
        }),
        serde_json::json!({
            "name": "ingest_wiki_source",
            "description": "触发 Wiki 源资料的摄入：自动解析文档 → 生成结构化 Wiki 页面 → 提取标签和 wikilinks。异步执行，可用 list_wiki_pages 检查进度。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "source_id": { "type": "string", "description": "源资料 ID" }
                },
                "required": ["project_id", "source_id"]
            }
        }),
        // ── Wiki tools: Project lifecycle ───────────────────────────
        serde_json::json!({
            "name": "create_wiki_project",
            "description": "创建新 Wiki 项目。Wiki 是结构化知识库，页面按 frontmatter 组织，支持标签、图谱、问答。创建后可通过 save_wiki_page 添加页面或 add_wiki_source 添加源资料。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Wiki 项目名称（1-100字符）" },
                    "description": { "type": "string", "description": "项目描述（可选）" },
                    "ingest_model": { "type": "string", "description": "摄入模型（用于自动生成页面的 LLM，可选）" },
                    "chat_model": { "type": "string", "description": "问答模型（用于 Wiki 问答的 LLM，可选）" },
                    "ingest_channel_id": { "type": "string", "description": "摄入渠道 ID（可选，默认自动选择）" },
                    "chat_channel_id": { "type": "string", "description": "问答渠道 ID（可选，默认自动选择）" },
                    "schema_text": { "type": "string", "description": "自定义 Wiki schema（CLAUDE.md 内容，可选）。定义页面结构规范、标签约定等。" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "delete_wiki_project",
            "description": "永久删除 Wiki 项目及其所有页面、源资料和目录。此操作不可恢复。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" }
                },
                "required": ["project_id"]
            }
        }),
        // ── Wiki tools: Page deletion ───────────────────────────────
        serde_json::json!({
            "name": "delete_wiki_page",
            "description": "删除 Wiki 项目中的指定页面。同时删除数据库记录和磁盘文件。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "path": { "type": "string", "description": "页面路径（如 'guides/old-page.md'）" }
                },
                "required": ["project_id", "path"]
            }
        }),
        // ── Wiki tools: Source management ──────────────────────────
        serde_json::json!({
            "name": "add_wiki_source",
            "description": "添加源资料到 Wiki 项目。可传入文件内容，自动保存到磁盘。添加后可用 ingest_wiki_source 触发摄入，自动生成结构化 Wiki 页面。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "filename": { "type": "string", "description": "源资料文件名（如 'design.md' 或 'service.java'）" },
                    "source_type": { "type": "string", "description": "源类型（如 'md', 'java', 'py', 'txt' 等）" },
                    "content": { "type": "string", "description": "文件内容（纯文本，非 base64）。如果提供，会自动保存到磁盘。" },
                    "file_path": { "type": "string", "description": "已有文件路径（可选，如果不提供 content 则用此路径）" },
                    "source_url": { "type": "string", "description": "来源 URL（可选）" }
                },
                "required": ["project_id", "filename", "source_type"]
            }
        }),
        serde_json::json!({
            "name": "delete_wiki_source",
            "description": "删除 Wiki 项目中的指定源资料。仅删除源记录，不影响已通过该源生成的 Wiki 页面。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string", "description": "Wiki 项目 ID" },
                    "source_id": { "type": "string", "description": "源资料 ID" }
                },
                "required": ["project_id", "source_id"]
            }
        }),
    ]
}

pub(crate) fn get_tools(principal: &AuthenticatedPrincipal) -> Vec<serde_json::Value> {
    let mut tools = tool_catalog();
    if !principal.allows(ACCESS_SCOPE_MCP_WRITE) {
        tools.retain(|tool| {
            tool.get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| !mcp_tool_requires_write(name))
        });
    }
    tools
}

pub(crate) fn get_tool_summaries() -> Vec<serde_json::Value> {
    tool_catalog()
        .into_iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?.to_string();
            let description = tool
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let label = match name.as_str() {
                "search_knowledge_base" => "语义搜索",
                "list_knowledge_bases" => "列出 RAG",
                "read_document" => "读取文档",
                "ask_knowledge_base" => "RAG 问答",
                "get_knowledge_base_stats" => "RAG 统计",
                "list_wiki_projects" => "列出 Wiki",
                "search_wiki" => "搜索 Wiki",
                "get_background_task" => "查询任务",
                "list_background_tasks" => "任务列表",
                _ => name.as_str(),
            }
            .to_string();
            let write = mcp_tool_requires_write(&name);
            Some(serde_json::json!({
                "name": name,
                "label": label,
                "desc": description,
                "write": write,
            }))
        })
        .collect()
}
