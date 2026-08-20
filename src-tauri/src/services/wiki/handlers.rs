use super::models::*;
use super::ingest;
use super::project;
use super::repository::WikiRepository;
use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::core::proxy;
use crate::db::repository::Repository;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize { 10 }

// ── Project handlers ──

pub async fn list_projects(State(shared): State<SharedState>) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_projects().await {
        Ok(projects) => Json(serde_json::json!({ "data": projects })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PROJECT_LIST_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }
}

pub async fn create_project(
    State(shared): State<SharedState>,
    Json(input): Json<CreateProjectInput>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    let project_id = project::new_uuid();
    let schema = input.schema_text.clone().unwrap_or_else(|| {
        super::repository::DEFAULT_SCHEMA.to_string()
    });

    // Create directory structure
    let dir = match project::init_project_dir(&project_id, &schema).await {
        Ok(dir) => dir,
        Err(error) => return HttpError::internal(
            "WIKI_DIRECTORY_CREATE_FAILED",
            "创建 Wiki 项目目录失败",
            error,
        ).into_response(),
    };
    let wiki_dir = dir.to_string_lossy().to_string();

    match repo.create_project(&input, &wiki_dir).await {
        Ok(p) => (StatusCode::CREATED, Json(p)).into_response(),
        Err(error) => {
            if let Err(cleanup_error) = project::remove_project_dir(&project_id).await {
                tracing::warn!(%cleanup_error, %project_id, "failed to clean up Wiki project directory");
            }
            HttpError::internal(
                "WIKI_PROJECT_CREATE_FAILED",
                "创建 Wiki 项目失败",
                error,
            ).into_response()
        }
    }
}

pub async fn get_project(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.find_project(&id).await {
        Ok(Some(project)) => Json(project).into_response(),
        Ok(None) => HttpError::not_found(
            "WIKI_PROJECT_NOT_FOUND",
            "Wiki 项目不存在",
        ).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PROJECT_READ_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }
}

pub async fn update_project(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateProjectInput>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());

    match repo.find_project(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return HttpError::not_found(
            "WIKI_PROJECT_NOT_FOUND",
            "Wiki 项目不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "WIKI_PROJECT_READ_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }

    // If schema_text changed, write to disk
    if let Some(ref schema) = input.schema_text {
        let dir = match project::project_wiki_dir(&id) {
            Ok(dir) => dir,
            Err(error) => return HttpError::bad_request(
                "WIKI_PROJECT_PATH_INVALID",
                error,
            ).into_response(),
        };
        let schema_path = dir.join("schema").join("CLAUDE.md");
        if let Err(e) = tokio::fs::write(&schema_path, schema).await {
            return HttpError::internal(
                "WIKI_SCHEMA_WRITE_FAILED",
                "保存 Wiki 结构定义失败",
                e,
            ).into_response();
        }
    }

    match repo.update_project(&id, &input).await {
        Ok(p) => Json(p).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PROJECT_UPDATE_FAILED",
            "更新 Wiki 项目失败",
            error,
        ).into_response(),
    }
}

pub async fn delete_project(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.find_project(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return HttpError::not_found(
            "WIKI_PROJECT_NOT_FOUND",
            "Wiki 项目不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "WIKI_PROJECT_READ_FAILED",
            "读取 Wiki 项目失败",
            error,
        ).into_response(),
    }
    let staged = match project::stage_project_dir_removal(&id).await {
        Ok(staged) => staged,
        Err(error) => return HttpError::internal(
            "WIKI_DIRECTORY_STAGE_FAILED",
            "准备删除 Wiki 项目目录失败",
            error,
        ).into_response(),
    };
    if let Err(error) = repo.delete_project(&id).await {
        if let Some(ref removal) = staged {
            if let Err(restore_error) = project::restore_staged_removal(removal).await {
                tracing::error!(%restore_error, %id, "failed to restore staged Wiki project directory");
            }
        }
        return HttpError::internal(
            "WIKI_PROJECT_DELETE_FAILED",
            "删除 Wiki 项目失败",
            error,
        ).into_response();
    }
    if let Some(removal) = staged {
        if let Err(error) = project::finalize_staged_removal(removal).await {
            tracing::warn!(%error, project_id = %id, "failed to finalize Wiki project directory deletion");
        }
    }
    (StatusCode::NO_CONTENT, "").into_response()
}

pub async fn get_project_stats(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.get_stats(&id).await {
        Ok(stats) => Json(stats).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_STATS_READ_FAILED",
            "读取 Wiki 统计失败",
            error,
        ).into_response(),
    }
}

// ── Source handlers ──

pub async fn list_sources(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_sources(&id).await {
        Ok(sources) => Json(serde_json::json!({ "data": sources })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_SOURCE_LIST_FAILED",
            "读取 Wiki 来源失败",
            error,
        ).into_response(),
    }
}

pub async fn add_source(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<AddSourceInput>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());

    let content_hash = input.content.as_ref().map(|c| {
        let mut hasher = Sha256::new();
        hasher.update(c.as_bytes());
        format!("{:x}", hasher.finalize())
    });

    let file_size = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);

    let written_path = if let Some(ref content) = input.content {
        match project::write_source_file(&id, &input.filename, content.as_bytes()).await {
            Ok(path) => Some(path),
            Err(error) => return HttpError::internal(
                "WIKI_SOURCE_FILE_WRITE_FAILED",
                "保存 Wiki 来源文件失败",
                error,
            ).into_response(),
        }
    } else {
        None
    };

    let mut persisted_input = input.clone();
    if let Some(path) = &written_path {
        persisted_input.file_path = Some(path.to_string_lossy().to_string());
    }

    match repo.add_source(&id, &persisted_input, content_hash.as_deref(), file_size).await {
        Ok(s) => (StatusCode::CREATED, Json(s)).into_response(),
        Err(error) => {
            if let Some(path) = written_path {
                if let Err(cleanup_error) = tokio::fs::remove_file(&path).await {
                    tracing::warn!(%cleanup_error, path = %path.display(), "failed to clean up Wiki source file");
                }
            }
            HttpError::internal(
                "WIKI_SOURCE_CREATE_FAILED",
                "创建 Wiki 来源失败",
                error,
            ).into_response()
        }
    }
}

pub async fn delete_source(
    State(shared): State<SharedState>,
    Path((id, sid)): Path<(String, String)>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    let source = match repo.find_source(&sid).await {
        Ok(Some(source)) if source.project_id == id => source,
        Ok(Some(_)) | Ok(None) => return HttpError::not_found(
            "WIKI_SOURCE_NOT_FOUND",
            "Wiki 来源不存在",
        ).into_response(),
        Err(error) => return HttpError::internal(
            "WIKI_SOURCE_READ_FAILED",
            "读取 Wiki 来源失败",
            error,
        ).into_response(),
    };
    let staged = match project::stage_source_file_removal(
        &id,
        &source.filename,
        source.file_path.as_deref(),
    ).await {
        Ok(staged) => staged,
        Err(error) => return HttpError::bad_request(
            "WIKI_SOURCE_PATH_INVALID",
            error,
        ).into_response(),
    };
    if let Err(error) = repo.delete_source(&sid).await {
        if let Some(ref removal) = staged {
            if let Err(restore_error) = project::restore_staged_removal(removal).await {
                tracing::error!(%restore_error, %sid, "failed to restore staged Wiki source file");
            }
        }
        return HttpError::internal(
            "WIKI_SOURCE_DELETE_FAILED",
            "删除 Wiki 来源失败",
            error,
        ).into_response();
    }
    if let Some(removal) = staged {
        if let Err(error) = project::finalize_staged_removal(removal).await {
            return HttpError::internal(
                "WIKI_SOURCE_FILE_DELETE_FAILED",
                "删除 Wiki 来源文件失败",
                error,
            ).into_response();
        }
    }
    (StatusCode::NO_CONTENT, "").into_response()
}

pub async fn ingest_source(
    State(shared): State<SharedState>,
    Path((id, sid)): Path<(String, String)>,
) -> Response {
    let app = shared.app.clone();
    let pool = shared.state.db.pool.clone();

    // Spawn ingest in background, return immediately with task info
    let project_id = id.clone();
    let source_id = sid.clone();

    match ingest::ingest_source(&app, &pool, &project_id, &source_id).await {
        Ok(result) => Json(serde_json::json!({
            "status": "done",
            "task_id": result.task_id,
            "pages_created": result.pages_created,
            "page_paths": result.page_paths,
        })).into_response(),
        Err(error) if error == ingest::INGEST_ALREADY_RUNNING => HttpError::conflict(
            "WIKI_INGEST_ALREADY_RUNNING",
            "该 Wiki 来源正在摄入",
        ).into_response(),
        Err(e) => {
            // Update source status to failed
            let repo = WikiRepository::new(pool);
            if let Err(error) = repo.update_source_status(&source_id, "failed", 0, Some(&e)).await {
                tracing::warn!(%error, source_id, "failed to persist Wiki source failure status");
            }
            HttpError::internal(
                "WIKI_INGEST_FAILED",
                "Wiki 来源摄入失败",
                e,
            ).into_response()
        }
    }
}

pub async fn rescan_sources(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let app = shared.app.clone();
    let pool = shared.state.db.pool.clone();
    let repo = WikiRepository::new(pool.clone());

    // Get all pending sources and ingest them
    let sources = match repo.list_sources(&id).await {
        Ok(s) => s,
        Err(error) => return HttpError::internal(
            "WIKI_SOURCE_LIST_FAILED",
            "读取 Wiki 来源失败",
            error,
        ).into_response(),
    };

    let pending: Vec<_> = sources
        .iter()
        .filter(|source| source.status == "pending" || source.status == "failed")
        .collect();
    let mut results = Vec::new();

    for source in &pending {
        match ingest::ingest_source(&app, &pool, &id, &source.id).await {
            Ok(r) => results.push(serde_json::json!({
                "source_id": source.id,
                "filename": source.filename,
                "status": "done",
                "pages": r.pages_created,
            })),
            Err(error) => {
                tracing::error!(%error, source_id = %source.id, "Wiki source rescan failed");
                if let Err(status_error) = repo
                    .update_source_status(&source.id, "failed", 0, Some("摄入失败"))
                    .await
                {
                    tracing::warn!(%status_error, source_id = %source.id, "failed to persist Wiki source failure status");
                }
                results.push(serde_json::json!({
                    "source_id": source.id,
                    "filename": source.filename,
                    "status": "failed",
                    "error": {
                        "code": "WIKI_INGEST_FAILED",
                        "message": "Wiki 来源摄入失败"
                    },
                }));
            }
        }
    }

    Json(serde_json::json!({
        "status": "done",
        "processed": pending.len(),
        "results": results,
    })).into_response()
}

// ── Page handlers ──

pub async fn list_pages(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_pages(&id).await {
        Ok(pages) => Json(serde_json::json!({ "data": pages })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PAGE_LIST_FAILED",
            "读取 Wiki 页面失败",
            error,
        ).into_response(),
    }
}

pub async fn get_page(
    State(shared): State<SharedState>,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());

    // Try DB first, but do not turn database failures into a misleading file fallback.
    match repo.get_page(&id, &path).await {
        Ok(Some(page)) => match project::snapshot_page(&id, &path).await {
            Ok(Some(content)) => {
                return Json(serde_json::json!({
                    "id": page.id,
                    "project_id": page.project_id,
                    "path": page.path,
                    "title": page.title,
                    "page_type": page.page_type,
                    "content_hash": page.content_hash,
                    "token_count": page.token_count,
                    "wikilinks": page.wikilinks,
                    "frontmatter": page.frontmatter,
                    "status": page.status,
                    "content": content,
                    "created_at": page.created_at,
                    "updated_at": page.updated_at,
                }))
                .into_response();
            }
            Ok(None) => {}
            Err(error) => return HttpError::internal(
                "WIKI_PAGE_FILE_READ_FAILED",
                "读取 Wiki 页面文件失败",
                error,
            ).into_response(),
        },
        Ok(None) => {}
        Err(error) => return HttpError::internal(
            "WIKI_PAGE_READ_FAILED",
            "读取 Wiki 页面失败",
            error,
        ).into_response(),
    }

    // Try reading file directly from disk
    match project::snapshot_page(&id, &path).await {
        Ok(Some(content)) => {
            let title = path.split('/').last().unwrap_or(&path)
                .trim_end_matches(".md").to_string();
            Json(serde_json::json!({
                "path": path,
                "title": title,
                "content": content,
                "page_type": "unknown",
            })).into_response()
        }
        Ok(None) => HttpError::not_found(
            "WIKI_PAGE_NOT_FOUND",
            "Wiki 页面不存在",
        ).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PAGE_FILE_READ_FAILED",
            "读取 Wiki 页面文件失败",
            error,
        ).into_response(),
    }
}

pub async fn update_page(
    State(shared): State<SharedState>,
    Path((id, path)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let content = body.get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match update_page_inner(&shared.state.db.pool, &repo, &id, &path, content).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "path": path })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_PAGE_UPDATE_FAILED",
            "保存 Wiki 页面失败",
            error,
        ).into_response(),
    }
}

/// Inner logic for saving a wiki page — shared by HTTP handler and MCP handler.
pub async fn update_page_inner(
    pool: &sqlx::SqlitePool,
    repo: &WikiRepository,
    id: &str,
    path: &str,
    content: &str,
) -> Result<(), String> {
    repo.get_project(id).await?;
    let previous = project::snapshot_page(id, path).await?;
    project::write_page(id, path, content).await?;

    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let title = ingest::extract_title_from_content(content, path);
    let page_type = if path.contains("entities/") { "entity" }
        else if path.contains("concepts/") { "concept" }
        else if path.contains("summaries/") { "summary" }
        else if path.ends_with("index.md") { "index" }
        else if path.ends_with("log.md") { "log" }
        else { "entity" };

    let token_count = (content.len() / 4) as i64; // rough estimate

    // Extract wikilinks
    let wikilinks: Vec<String> = extract_wikilinks(content);
    let wikilinks_json = serde_json::to_string(&wikilinks).unwrap_or("[]".to_string());

    // Extract tags from frontmatter
    let tags = crate::services::wiki::ingest::extract_tags_from_frontmatter(content);
    let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
    let frontmatter = crate::services::wiki::ingest::extract_frontmatter(content).unwrap_or("{}");

    if let Err(error) = repo.upsert_page(id, path, &title, page_type, &hash, token_count, &wikilinks_json, frontmatter, &tags_json).await {
        if let Err(rollback_error) = project::rollback_page(id, path, previous.as_deref()).await {
            return Err(format!("{}; failed to restore page file: {}", error, rollback_error));
        }
        return Err(error);
    }

    // Rebuild knowledge graph edges based on updated wikilinks
    if let Err(error) = ingest::rebuild_graph_edges(pool, id).await {
        tracing::warn!(%error, project_id = %id, page_path = %path, "failed to rebuild Wiki graph after page save");
    }

    // Append log
    if let Err(error) = project::append_log(id, &format!("update | {}", path)).await {
        tracing::warn!(%error, project_id = %id, page_path = %path, "failed to append Wiki page log");
    }

    Ok(())
}

pub async fn delete_page(
    State(shared): State<SharedState>,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    let staged = match project::stage_page_file_removal(&id, &path).await {
        Ok(staged) => staged,
        Err(error) => return HttpError::bad_request(
            "WIKI_PAGE_PATH_INVALID",
            error,
        ).into_response(),
    };
    if let Err(error) = repo.delete_page(&id, &path).await {
        if let Some(ref removal) = staged {
            if let Err(restore_error) = project::restore_staged_removal(removal).await {
                tracing::error!(%restore_error, %id, %path, "failed to restore staged Wiki page file");
            }
        }
        return HttpError::internal(
            "WIKI_PAGE_DELETE_FAILED",
            "删除 Wiki 页面失败",
            error,
        ).into_response();
    }
    if let Some(removal) = staged {
        if let Err(error) = project::finalize_staged_removal(removal).await {
            tracing::warn!(%error, project_id = %id, page_path = %path, "failed to finalize Wiki page file deletion");
        }
    }
    // Rebuild graph edges after page deletion
    if let Err(error) = ingest::rebuild_graph_edges(&shared.state.db.pool, &id).await {
        tracing::warn!(%error, project_id = %id, page_path = %path, "failed to rebuild Wiki graph after page deletion");
    }
    (StatusCode::NO_CONTENT, "").into_response()
}

// ── Search & Ask ──

pub async fn search(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Query(params): Query<SearchQuery>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.search_pages(&id, &params.q, params.top_k).await {
        Ok(results) => Json(serde_json::json!({ "data": results, "query": params.q })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_SEARCH_FAILED",
            "搜索 Wiki 失败",
            error,
        ).into_response(),
    }
}

pub async fn ask(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<WikiAskInput>,
) -> Response {
    let top_k = input.top_k.unwrap_or(5);
    let model = input.model.as_deref();

    match ask_inner(&shared, &id, &input.question, top_k, model).await {
        Ok(json) => Json(json).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_ASK_FAILED",
            "Wiki 问答失败",
            error,
        ).into_response(),
    }
}

/// Inner logic for Wiki Q&A — shared by HTTP handler and MCP handler.
pub async fn ask_inner(
    shared: &SharedState,
    id: &str,
    question: &str,
    top_k: usize,
    model_override: Option<&str>,
) -> Result<serde_json::Value, String> {
    let pool = &shared.state.db.pool;
    let repo = WikiRepository::new(pool.clone());
    let db_repo = Arc::new(Repository::new(pool.clone()));
    let app = shared.app.clone();

    // Search relevant pages
    let results = repo.search_pages(id, question, top_k).await?;

    // Read page contents
    let mut contexts = Vec::new();
    for r in &results {
        match project::read_page(id, &r.path).await {
            Ok(content) => {
                let snippet: String = content.chars().take(2000).collect();
                contexts.push(format!("## {} ({})\n{}", r.title, r.path, snippet));
            }
            Err(error) => {
                tracing::warn!(%error, project_id = %id, page_path = %r.path, "failed to read Wiki search result");
            }
        }
    }

    if contexts.is_empty() {
        return Ok(serde_json::json!({
            "answer": "No relevant wiki pages found for your question. Please ingest some documents first.",
            "sources": []
        }));
    }

    let context_text = contexts.join("\n\n---\n\n");

    // Get project config
    let proj = repo.get_project(id).await?;

    let chat_model = model_override
        .or(proj.chat_model.as_deref())
        .unwrap_or("gpt-4o");
    let chat_channel_id = match proj.chat_channel_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            let row: Option<(String,)> = sqlx::query_as("SELECT id FROM channels WHERE status = 1 ORDER BY priority DESC LIMIT 1")
                .fetch_optional(pool).await
                .map_err(|e| format!("DB error: {}", e))?;
            match row.map(|(id,)| id) {
                Some(id) => id,
                None => return Err("No active channel configured. Please create a channel first or set chat_channel_id in Wiki project settings.".to_string()),
            }
        }
    };

    let system_prompt = "You are a Wiki knowledge assistant. Answer questions based on the provided wiki pages. Be concise and cite source pages using [[wikilinks]] format.";
    let user_prompt = format!(
        "Based on the following wiki pages, answer the question.\n\nWiki pages:\n{}\n\nQuestion: {}\n\nAnswer:",
        context_text, question
    );

    // Save user message
    if let Err(error) = repo.add_session(id, "user", question, None, None).await {
        tracing::warn!(%error, project_id = %id, "failed to persist Wiki user message");
    }

    let chat_request = serde_json::json!({
        "model": chat_model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ],
        "stream": false,
        "temperature": 0.4
    });
    let chat_request_str: String = serde_json::to_string(&chat_request).unwrap_or_default();

    let proxy_result = proxy::handle_request(
        &db_repo,
        &app,
        &chat_channel_id,
        "Wiki Chat",
        chat_request,
        false,
        "chat",
        Some(chat_request_str),
        Some(format!("wiki-chat_{}", id)),
        None,
    ).await;

    let (answer, usage) = match proxy_result {
        Ok(result) => {
            let answer_text = result.body
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|t| t.as_str())
                .unwrap_or("Failed to generate answer.");

            let usage = result.body.get("usage").map(|u| serde_json::json!({
                "prompt_tokens": u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "completion_tokens": u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "total_tokens": u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            }));

            (answer_text.to_string(), usage)
        }
        Err((code, msg)) => {
            let err_answer = format!("LLM request failed ({}): {}", code, msg);
            (err_answer, None)
        }
    };

    let sources: Vec<WikiAnswerSource> = results.iter().map(|r| WikiAnswerSource {
        path: r.path.clone(),
        title: r.title.clone(),
        score: r.score,
        snippet: r.snippet.clone(),
    }).collect();

    // Save assistant message
    if let Err(error) = repo
        .add_session(
            id,
            "assistant",
            &answer,
            Some(&serde_json::to_string(&sources).unwrap_or_default()),
            Some(chat_model),
        )
        .await
    {
        tracing::warn!(%error, project_id = %id, "failed to persist Wiki assistant message");
    }

    Ok(serde_json::json!({
        "answer": answer,
        "sources": sources,
        "usage": usage,
    }))
}

// ── Graph ──

pub async fn get_graph(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.get_graph(&id).await {
        Ok(graph) => Json(graph).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_GRAPH_READ_FAILED",
            "读取 Wiki 知识图谱失败",
            error,
        ).into_response(),
    }
}

// ── Sessions ──

pub async fn list_sessions(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_sessions(&id).await {
        Ok(sessions) => Json(serde_json::json!({ "data": sessions })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_SESSION_LIST_FAILED",
            "读取 Wiki 会话失败",
            error,
        ).into_response(),
    }
}

pub async fn clear_sessions(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    if let Err(error) = repo.clear_sessions(&id).await {
        return HttpError::internal(
            "WIKI_SESSION_CLEAR_FAILED",
            "清空 Wiki 会话失败",
            error,
        ).into_response();
    }
    (StatusCode::NO_CONTENT, "").into_response()
}

// ── Queue ──

pub async fn get_queue_status(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.list_tasks(&id).await {
        Ok(tasks) => Json(serde_json::json!({ "data": tasks })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_TASK_LIST_FAILED",
            "读取 Wiki 任务失败",
            error,
        ).into_response(),
    }
}

// ── Helpers ──

fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut start = 0;
    loop {
        if let Some(s) = content[start..].find("[[") {
            let s = start + s + 2;
            if let Some(e) = content[s..].find("]]") {
                let link = &content[s..s + e];
                if !link.is_empty() {
                    links.push(link.to_string());
                }
                start = s + e + 2;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    links
}
