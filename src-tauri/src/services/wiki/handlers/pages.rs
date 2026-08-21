use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::services::wiki::{ingest, project, repository::WikiRepository};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use sha2::{Digest, Sha256};

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

    if let Err(error) = repo.upsert_page(id, path, &title, page_type, &hash, token_count, &wikilinks_json, frontmatter, &tags_json, content).await {
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

