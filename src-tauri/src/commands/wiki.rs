#![allow(non_snake_case)] // Tauri argument names are part of the frontend command contract.

use crate::AppState;
use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tauri::State;

// Re-export models for convenience
pub use crate::services::wiki::models::*;

// ── Wiki Project commands ──

#[tauri::command]
pub async fn get_wiki_projects(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<Vec<WikiProject>> {
    let pool = state.db.pool.clone();
    let rows = sqlx::query_as::<_, WikiProject>(
        "SELECT * FROM wiki_projects ORDER BY created_at DESC"
    )
    .fetch_all(&pool)
    .await
    .command_error("WIKI_PROJECT_LIST_FAILED", "读取 Wiki 项目失败", true)?;
    Ok(rows)
}

#[tauri::command]
pub async fn create_wiki_project(
    state: State<'_, Arc<AppState>>,
    input: CreateProjectInput,
) -> CommandResult<WikiProject> {
    let pool = state.db.pool.clone();
    let project_id = crate::services::wiki::project::new_uuid();
    let schema = input.schema_text.clone().unwrap_or_else(|| {
        crate::services::wiki::repository::DEFAULT_SCHEMA.to_string()
    });

    // Create directory structure
    let dir = crate::services::wiki::project::init_project_dir(&project_id, &schema)
        .await
        .command_error("WIKI_PROJECT_STORAGE_CREATE_FAILED", "创建 Wiki 项目目录失败", false)?;
    let wiki_dir = dir.to_string_lossy().to_string();

    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    match repo.create_project(&input, &wiki_dir).await {
        Ok(project) => Ok(project),
        Err(error) => {
            if let Err(cleanup_error) = crate::services::wiki::project::remove_project_dir(&project_id).await {
                return Err(CommandError::reported(
                    "WIKI_PROJECT_CREATE_ROLLBACK_FAILED",
                    "创建 Wiki 项目失败，且无法清理项目目录",
                    false,
                    format!("{}; cleanup: {}", error, cleanup_error),
                ));
            }
            Err(CommandError::reported(
                "WIKI_PROJECT_CREATE_FAILED",
                "创建 Wiki 项目失败",
                false,
                error,
            ))
        }
    }
}

#[tauri::command]
pub async fn get_wiki_project(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CommandResult<WikiProject> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    repo.get_project(&id)
        .await
        .command_error("WIKI_PROJECT_READ_FAILED", "读取 Wiki 项目失败", true)
}

#[tauri::command]
pub async fn update_wiki_project(
    state: State<'_, Arc<AppState>>,
    id: String,
    input: UpdateProjectInput,
) -> CommandResult<WikiProject> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);

    if let Some(ref schema) = input.schema_text {
        let dir = crate::services::wiki::project::project_wiki_dir(&id)
            .command_error("WIKI_PROJECT_PATH_INVALID", "Wiki 项目路径无效", false)?;
        let schema_path = dir.join("schema").join("CLAUDE.md");
        tokio::fs::write(&schema_path, schema)
            .await
            .command_error("WIKI_SCHEMA_WRITE_FAILED", "保存 Wiki Schema 失败", false)?;
    }

    repo.update_project(&id, &input)
        .await
        .command_error("WIKI_PROJECT_UPDATE_FAILED", "更新 Wiki 项目失败", false)
}

#[tauri::command]
pub async fn delete_wiki_project(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CommandResult<()> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    let staged = crate::services::wiki::project::stage_project_dir_removal(&id)
        .await
        .command_error("WIKI_PROJECT_DELETE_STAGE_FAILED", "准备删除 Wiki 项目失败", false)?;
    if let Err(error) = repo.delete_project(&id).await {
        if let Some(ref removal) = staged {
            if let Err(restore_error) = crate::services::wiki::project::restore_staged_removal(removal).await {
                return Err(CommandError::reported(
                    "WIKI_PROJECT_DELETE_ROLLBACK_FAILED",
                    "删除 Wiki 项目失败，且无法恢复项目目录",
                    false,
                    format!("{}; restore: {}", error, restore_error),
                ));
            }
        }
        return Err(CommandError::reported(
            "WIKI_PROJECT_DELETE_FAILED",
            "删除 Wiki 项目失败",
            false,
            error,
        ));
    }
    if let Some(removal) = staged {
        if let Err(error) = crate::services::wiki::project::finalize_staged_removal(removal).await {
            tracing::warn!(%error, project_id = %id, "failed to finalize Wiki project directory deletion");
        }
    }
    Ok(())
}

// ── Wiki Pages ──

#[tauri::command]
pub async fn get_wiki_pages(
    state: State<'_, Arc<AppState>>,
    projectId: String,
) -> CommandResult<Vec<WikiPage>> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    repo.list_pages(&projectId)
        .await
        .command_error("WIKI_PAGE_LIST_FAILED", "读取 Wiki 页面失败", true)
}

#[tauri::command]
pub async fn get_wiki_page(
    state: State<'_, Arc<AppState>>,
    projectId: String,
    path: String,
) -> CommandResult<serde_json::Value> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);

    match repo
        .get_page(&projectId, &path)
        .await
        .command_error("WIKI_PAGE_READ_FAILED", "读取 Wiki 页面失败", true)?
    {
        Some(page) => {
            let content = crate::services::wiki::project::read_page(&projectId, &path)
                .await
                .command_error("WIKI_PAGE_FILE_READ_FAILED", "读取 Wiki 页面文件失败", true)?;
            return Ok(serde_json::json!({
                "id": page.id,
                "project_id": page.project_id,
                "path": page.path,
                "title": page.title,
                "page_type": page.page_type,
                "content": content,
                "wikilinks": page.wikilinks,
                "frontmatter": page.frontmatter,
                "tags": page.tags,
                "status": page.status,
                "created_at": page.created_at,
                "updated_at": page.updated_at,
            }));
        }
        None => {}
    }

    // Try reading from disk
    match crate::services::wiki::project::read_page(&projectId, &path).await {
        Ok(content) => {
            let title = path.split('/').last().unwrap_or(&path).trim_end_matches(".md").to_string();
            Ok(serde_json::json!({
                "path": path,
                "title": title,
                "content": content,
                "page_type": "unknown",
            }))
        }
        Err(error) => Err(CommandError::reported(
            "WIKI_PAGE_NOT_FOUND",
            "Wiki 页面不存在或无法读取",
            false,
            error,
        )),
    }
}

#[tauri::command]
pub async fn save_wiki_page(
    state: State<'_, Arc<AppState>>,
    projectId: String,
    path: String,
    content: String,
) -> CommandResult<()> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    crate::services::wiki::handlers::update_page_inner(
        &state.db.pool,
        &repo,
        &projectId,
        &path,
        &content,
    ).await.command_error("WIKI_PAGE_SAVE_FAILED", "保存 Wiki 页面失败", false)
}

// ── Wiki Sources ──

#[tauri::command]
pub async fn get_wiki_sources(
    state: State<'_, Arc<AppState>>,
    projectId: String,
) -> CommandResult<Vec<WikiSource>> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    repo.list_sources(&projectId)
        .await
        .command_error("WIKI_SOURCE_LIST_FAILED", "读取 Wiki 来源失败", true)
}

#[tauri::command]
pub async fn add_wiki_source(
    state: State<'_, Arc<AppState>>,
    projectId: String,
    input: AddSourceInput,
) -> CommandResult<WikiSource> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);

    let content_hash = input.content.as_ref().map(|c| {
        let mut hasher = Sha256::new();
        hasher.update(c.as_bytes());
        format!("{:x}", hasher.finalize())
    });
    let file_size = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);

    let written_path = if let Some(ref content) = input.content {
        Some(crate::services::wiki::project::write_source_file(
            &projectId,
            &input.filename,
            content.as_bytes(),
        ).await.command_error("WIKI_SOURCE_FILE_WRITE_FAILED", "保存 Wiki 来源文件失败", false)?)
    } else {
        None
    };

    let mut persisted_input = input.clone();
    if let Some(path) = &written_path {
        persisted_input.file_path = Some(path.to_string_lossy().to_string());
    }

    match repo.add_source(&projectId, &persisted_input, content_hash.as_deref(), file_size).await {
        Ok(source) => Ok(source),
        Err(error) => {
            if let Some(path) = written_path {
                if let Err(cleanup_error) = tokio::fs::remove_file(&path).await {
                    return Err(CommandError::reported(
                        "WIKI_SOURCE_CREATE_ROLLBACK_FAILED",
                        "创建 Wiki 来源失败，且无法清理来源文件",
                        false,
                        format!("{}; cleanup: {}", error, cleanup_error),
                    ));
                }
            }
            Err(CommandError::reported(
                "WIKI_SOURCE_CREATE_FAILED",
                "创建 Wiki 来源失败",
                false,
                error,
            ))
        }
    }
}

#[tauri::command]
pub async fn delete_wiki_source(
    state: State<'_, Arc<AppState>>,
    sourceId: String,
) -> CommandResult<()> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    let source = repo
        .get_source(&sourceId)
        .await
        .command_error("WIKI_SOURCE_READ_FAILED", "读取 Wiki 来源失败", true)?;
    let project_id = source.project_id.clone();
    let staged = crate::services::wiki::project::stage_source_file_removal(
        &project_id,
        &source.filename,
        source.file_path.as_deref(),
    ).await.command_error("WIKI_SOURCE_DELETE_STAGE_FAILED", "准备删除 Wiki 来源失败", false)?;
    if let Err(error) = repo.delete_source(&sourceId).await {
        if let Some(ref removal) = staged {
            if let Err(restore_error) = crate::services::wiki::project::restore_staged_removal(removal).await {
                return Err(CommandError::reported(
                    "WIKI_SOURCE_DELETE_ROLLBACK_FAILED",
                    "删除 Wiki 来源失败，且无法恢复来源文件",
                    false,
                    format!("{}; restore: {}", error, restore_error),
                ));
            }
        }
        return Err(CommandError::reported(
            "WIKI_SOURCE_DELETE_FAILED",
            "删除 Wiki 来源失败",
            false,
            error,
        ));
    }
    if let Some(removal) = staged {
        crate::services::wiki::project::finalize_staged_removal(removal)
            .await
            .command_error("WIKI_SOURCE_DELETE_FINALIZE_FAILED", "清理 Wiki 来源文件失败", false)?;
    }
    Ok(())
}

// ── Wiki Search ──

#[tauri::command]
pub async fn search_wiki(
    state: State<'_, Arc<AppState>>,
    projectId: String,
    query: String,
    topK: Option<usize>,
) -> CommandResult<Vec<WikiSearchResult>> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    repo.search_pages(&projectId, &query, topK.unwrap_or(10))
        .await
        .command_error("WIKI_SEARCH_FAILED", "搜索 Wiki 页面失败", true)
}

// ── Wiki Graph ──

#[tauri::command]
pub async fn get_wiki_graph(
    state: State<'_, Arc<AppState>>,
    projectId: String,
) -> CommandResult<GraphData> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    repo.get_graph(&projectId)
        .await
        .command_error("WIKI_GRAPH_FAILED", "读取 Wiki 关系图失败", true)
}

// ── Wiki Tags ──

#[tauri::command]
pub async fn get_wiki_tags(
    state: State<'_, Arc<AppState>>,
    projectId: String,
    limit: Option<usize>,
) -> CommandResult<Vec<WikiTag>> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    repo.get_tags(&projectId, limit.unwrap_or(15))
        .await
        .command_error("WIKI_TAGS_FAILED", "读取 Wiki 标签失败", true)
}

// ── Wiki Stats ──

#[tauri::command]
pub async fn get_wiki_stats(
    state: State<'_, Arc<AppState>>,
    projectId: String,
) -> CommandResult<serde_json::Value> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool);
    repo.get_stats(&projectId)
        .await
        .command_error("WIKI_STATS_FAILED", "读取 Wiki 统计失败", true)
}

// ── Wiki Ingest ──

#[tauri::command]
pub async fn ingest_wiki_source(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    projectId: String,
    sourceId: String,
) -> CommandResult<serde_json::Value> {
    let pool = state.db.pool.clone();
    crate::services::wiki::ingest::ingest_source(&app, &pool, &projectId, &sourceId).await
        .map(|r| serde_json::json!({
            "status": "done",
            "task_id": r.task_id,
            "pages_created": r.pages_created,
            "page_paths": r.page_paths,
        }))
        .map_err(|error| {
            if error == crate::services::wiki::ingest::INGEST_ALREADY_RUNNING {
                CommandError::new(
                    "WIKI_INGEST_ALREADY_RUNNING",
                    "该 Wiki 来源正在摄入",
                    true,
                )
            } else {
                CommandError::reported(
                    "WIKI_INGEST_FAILED",
                    "Wiki 来源摄入失败",
                    true,
                    error,
                )
            }
        })
}

#[tauri::command]
pub async fn rescan_wiki_sources(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    projectId: String,
) -> CommandResult<serde_json::Value> {
    let pool = state.db.pool.clone();
    let repo = crate::services::wiki::repository::WikiRepository::new(pool.clone());

    let sources = repo
        .list_sources(&projectId)
        .await
        .command_error("WIKI_SOURCE_LIST_FAILED", "读取 Wiki 来源失败", true)?;
    let pending: Vec<_> = sources
        .iter()
        .filter(|source| matches!(source.status.as_str(), "pending" | "failed"))
        .collect();
    let mut results = Vec::new();

    for source in &pending {
        match crate::services::wiki::ingest::ingest_source(&app, &pool, &projectId, &source.id).await {
            Ok(r) => results.push(serde_json::json!({
                "source_id": source.id,
                "filename": source.filename,
                "status": "done",
                "pages": r.pages_created,
            })),
            Err(error) => {
                tracing::error!(%error, source_id = %source.id, "Wiki source rescan failed");
                results.push(serde_json::json!({
                    "source_id": source.id,
                    "filename": source.filename,
                    "status": "failed",
                    "error": {
                        "code": if error == crate::services::wiki::ingest::INGEST_ALREADY_RUNNING {
                            "WIKI_INGEST_ALREADY_RUNNING"
                        } else {
                            "WIKI_INGEST_FAILED"
                        },
                        "message": if error == crate::services::wiki::ingest::INGEST_ALREADY_RUNNING {
                            "该 Wiki 来源正在摄入"
                        } else {
                            "Wiki 来源摄入失败"
                        }
                    }
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "status": "done",
        "processed": pending.len(),
        "results": results,
    }))
}
