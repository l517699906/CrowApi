use super::{IngestResult, WrittenPage};
use crate::db::repository::Repository;
use crate::services::wiki::{models::WikiSource, project, repository::WikiRepository};
use crate::services::tasks::{
    emit_task_event,
    models::TASK_CANCELLED,
    repository::TaskRepository,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

pub const INGEST_ALREADY_RUNNING: &str = "WIKI_INGEST_ALREADY_RUNNING";

/// Ingest a source file: read → parse → generate wiki pages via LLM → write to disk+DB.
pub async fn ingest_source(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    project_id: &str,
    source_id: &str,
) -> Result<IngestResult, String> {
    let (source, task_id) = prepare_ingest(pool, project_id, source_id).await?;
    run_ingest_task(app, pool, project_id, source_id, source, task_id).await
}

pub async fn start_ingest_source(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    project_id: &str,
    source_id: &str,
) -> Result<String, String> {
    let (source, task_id) = prepare_ingest(pool, project_id, source_id).await?;
    spawn_ingest_task(
        app.clone(),
        pool.clone(),
        project_id.to_string(),
        source_id.to_string(),
        source,
        task_id.clone(),
    );
    Ok(task_id)
}

pub async fn start_existing_ingest_task(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    task_id: &str,
) -> Result<(), String> {
    let task = TaskRepository::new(pool.clone())
        .get(task_id)
        .await
        .map_err(|error| error.to_string())?;
    if task.domain != "wiki"
        || task.task_type != "ingest"
        || task.resource_type != "wiki_project"
        || task.status != "pending"
    {
        return Err("后台任务不是可执行的 Wiki 摄取任务".to_string());
    }
    let payload: serde_json::Value = serde_json::from_str(&task.payload_json)
        .map_err(|error| format!("Wiki 摄取任务参数损坏: {}", error))?;
    if payload.get("payload_version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("Wiki 摄取任务参数版本不受支持".to_string());
    }
    let source_id = task
        .subject_id
        .clone()
        .ok_or_else(|| "Wiki 摄取任务缺少来源标识".to_string())?;
    if payload.get("project_id").and_then(serde_json::Value::as_str)
        != Some(task.resource_id.as_str())
        || payload.get("source_id").and_then(serde_json::Value::as_str)
            != Some(source_id.as_str())
    {
        return Err("Wiki 摄取任务参数与资源不匹配".to_string());
    }
    let source = load_source(pool, &task.resource_id, &source_id).await?;
    spawn_ingest_task(
        app.clone(),
        pool.clone(),
        task.resource_id,
        source_id,
        source,
        task.id,
    );
    Ok(())
}

async fn prepare_ingest(
    pool: &sqlx::SqlitePool,
    project_id: &str,
    source_id: &str,
) -> Result<(WikiSource, String), String> {
    let repo = WikiRepository::new(pool.clone());
    let source = load_source(pool, project_id, source_id).await?;
    let task_id = repo
        .create_task_if_idle(project_id, source_id, "ingest")
        .await?
        .ok_or_else(|| INGEST_ALREADY_RUNNING.to_string())?;
    Ok((source, task_id))
}

async fn load_source(
    pool: &sqlx::SqlitePool,
    project_id: &str,
    source_id: &str,
) -> Result<WikiSource, String> {
    WikiRepository::new(pool.clone())
        .get_source(source_id)
        .await
        .and_then(|source| {
            if source.project_id == project_id {
                Ok(source)
            } else {
                Err(format!("Source {} not found", source_id))
            }
        })
}

fn spawn_ingest_task(
    app: AppHandle,
    pool: sqlx::SqlitePool,
    project_id: String,
    source_id: String,
    source: WikiSource,
    task_id: String,
) {
    tokio::spawn(async move {
        if let Err(error) = run_ingest_task(
            &app,
            &pool,
            &project_id,
            &source_id,
            source,
            task_id,
        )
        .await
        {
            tracing::error!(%error, project_id, source_id, "Wiki background ingest failed");
        }
    });
}

async fn run_ingest_task(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    project_id: &str,
    source_id: &str,
    source: WikiSource,
    task_id: String,
) -> Result<IngestResult, String> {
    let repo = WikiRepository::new(pool.clone());
    let db_repo = Arc::new(Repository::new(pool.clone()));
    repo.update_task_status(&task_id, "running", 0, 0, 3, None, None).await?;
    repo.update_source_status(source_id, "processing", 0, None).await?;
    emit_current_task(app, pool, &task_id, Some("准备摄取 Wiki 来源")).await;
    emit_wiki_progress(app, source_id, project_id, &source.filename, "processing", 0, "准备摄入");

    let result = run_ingest_source(
        app,
        pool,
        project_id,
        source_id,
        &source,
        &task_id,
        &repo,
        &db_repo,
    )
    .await;

    if let Err(error) = &result {
        tracing::error!(%error, project_id, source_id, task_id, "Wiki ingest failed");
        if error == TASK_CANCELLED {
            if let Err(status_error) = repo
                .update_source_status(source_id, "cancelled", 0, None)
                .await
            {
                tracing::warn!(%status_error, project_id, source_id, "failed to persist cancelled Wiki source");
            }
            emit_wiki_progress(app, source_id, project_id, &source.filename, "cancelled", 0, "已取消");
        } else {
            if let Err(status_error) = repo
                .update_task_status(
                    &task_id,
                    "failed",
                    0,
                    0,
                    3,
                    None,
                    Some("Wiki 来源摄入失败"),
                )
                .await
            {
                tracing::warn!(%status_error, project_id, source_id, task_id, "failed to persist Wiki ingest task failure");
            }
            if let Err(status_error) = repo
                .update_source_status(source_id, "failed", 0, Some("Wiki 来源摄入失败"))
                .await
            {
                tracing::warn!(%status_error, project_id, source_id, "failed to persist Wiki source failure");
            }
            emit_wiki_progress(app, source_id, project_id, &source.filename, "failed", 0, "摄入失败");
        }
        emit_current_task(app, pool, &task_id, Some(error)).await;
    } else {
        emit_current_task(app, pool, &task_id, Some("Wiki 摄取完成")).await;
    }

    result
}

async fn emit_current_task(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    task_id: &str,
    detail: Option<&str>,
) {
    if let Ok(task) = TaskRepository::new(pool.clone()).get(task_id).await {
        emit_task_event(app, &task, detail);
    }
}

async fn run_ingest_source(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    project_id: &str,
    source_id: &str,
    source: &WikiSource,
    task_id: &str,
    repo: &WikiRepository,
    db_repo: &Arc<Repository>,
) -> Result<IngestResult, String> {
    let tasks = TaskRepository::new(pool.clone());
    tasks.ensure_not_cancelled(task_id).await?;

    // 3. Read source file content
    let content = if let Some(ref file_path) = source.file_path {
        // Read from disk path
        tokio::fs::read_to_string(file_path).await
            .map_err(|e| format!("Failed to read source file: {}", e))?
    } else {
        // Read from project raw/sources/ dir
        let raw = project::read_source_file(project_id, &source.filename).await
            .map_err(|e| format!("Failed to read source: {}", e))?;
        String::from_utf8_lossy(&raw).to_string()
    };

    let source_filename = &source.filename;
    let file_ext = source_filename.rsplit('.').next().unwrap_or("txt").to_lowercase();

    // 4. Parse content into sections/chunks
    repo.update_task_status(&task_id, "running", 10, 0, 3, None, None).await?;
    emit_current_task(app, pool, task_id, Some("解析 Wiki 来源")).await;
    emit_wiki_progress(app, source_id, project_id, &source.filename, "parsing", 10, "解析文档");
    let sections = super::parser::parse_content(&content, &file_ext);
    tasks.ensure_not_cancelled(task_id).await?;

    // 5. Get project config for LLM
    let proj = repo.get_project(project_id).await?;
    let ingest_model = proj.ingest_model.as_deref().unwrap_or("gpt-4o");
    let ingest_channel_id = match proj.ingest_channel_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            // Fallback: find first active channel from DB
            let row: Option<(String,)> = sqlx::query_as("SELECT id FROM channels WHERE status = 1 ORDER BY priority DESC LIMIT 1")
                .fetch_optional(pool).await
                .map_err(|e| format!("DB error: {}", e))?;
            let id = row.map(|(id,)| id).ok_or_else(|| "No active channel configured. Please create a channel first or set ingest_channel_id in Wiki project settings.".to_string())?;
            id
        }
    };

    // 6. Generate wiki pages via LLM
    repo.update_task_status(&task_id, "running", 30, 1, 3, None, None).await?;
    emit_current_task(app, pool, task_id, Some("调用模型生成 Wiki 页面")).await;
    emit_wiki_progress(app, source_id, project_id, &source.filename, "generating", 30, "LLM 生成页面");
    let pages = super::generator::generate_wiki_pages(
        app,
        db_repo,
        ingest_model,
        &ingest_channel_id,
        project_id,
        source_filename,
        &sections,
        proj.schema_text.as_deref().unwrap_or(crate::services::wiki::repository::DEFAULT_SCHEMA),
    ).await?;
    tasks.ensure_not_cancelled(task_id).await?;

    // 7. Write pages to disk + DB
    repo.update_task_status(&task_id, "running", 60, 2, 3, None, None).await?;
    emit_current_task(app, pool, task_id, Some("写入 Wiki 页面")).await;
    emit_wiki_progress(app, source_id, project_id, &source.filename, "writing", 60, "写入页面");
    let mut written_pages = Vec::new();
    for page in &pages {
        tasks.ensure_not_cancelled(task_id).await?;
        let page_path = &page.path;

        // Write to disk
        project::write_page(project_id, page_path, &page.content).await?;

        // Compute hash
        let mut hasher = Sha256::new();
        hasher.update(page.content.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let token_count = (page.content.len() / 4) as i64;

        // Extract wikilinks
        let wikilinks = super::graph::extract_wikilinks(&page.content);
        let wikilinks_json = serde_json::to_string(&wikilinks).unwrap_or_else(|_| "[]".to_string());

        // Extract tags from frontmatter
        let tags = super::graph::extract_tags_from_frontmatter(&page.content);
        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
        let frontmatter = super::graph::extract_frontmatter(&page.content).unwrap_or("{}");

        // Upsert into DB
        repo.upsert_page(
            project_id,
            page_path,
            &page.title,
            &page.page_type,
            &hash,
            token_count,
            &wikilinks_json,
            frontmatter,
            &tags_json,
            &page.content,
        ).await?;

        written_pages.push(WrittenPage {
            path: page_path.clone(),
            wikilinks,
        });
    }

    // 8. Update graph edges from wikilinks
    tasks.ensure_not_cancelled(task_id).await?;
    repo.update_task_status(&task_id, "running", 80, 2, 3, None, None).await?;
    emit_current_task(app, pool, task_id, Some("更新 Wiki 知识图谱")).await;
    emit_wiki_progress(app, source_id, project_id, &source.filename, "linking", 80, "更新知识图谱");
    super::graph::update_graph_edges(pool, project_id, &written_pages).await?;

    // 9. Update source status
    tasks.ensure_not_cancelled(task_id).await?;
    repo.update_source_status(source_id, "ingested", written_pages.len() as i64, None).await?;

    // Update project counts
    let page_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM wiki_pages WHERE project_id = ? AND status = 'active'"
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let source_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM wiki_sources WHERE project_id = ? AND status = 'ingested'"
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE wiki_projects SET page_count=?, source_count=?, last_ingest_at=?, updated_at=? WHERE id=?"
    )
    .bind(page_count).bind(source_count).bind(&now).bind(&now).bind(project_id)
    .execute(pool).await.map_err(|e| format!("DB error: {}", e))?;

    // Append log
    if let Err(error) = project::append_log(
        project_id,
        &format!("ingest | {} → {} pages", source_filename, written_pages.len()),
    )
    .await
    {
        tracing::warn!(%error, project_id, "failed to append Wiki ingest log");
    }

    // Update task
    let result_json = serde_json::json!({
        "pages_created": written_pages.len(),
        "source": source_filename,
    }).to_string();
    repo.update_task_status(&task_id, "done", 100, 3, 3, Some(&result_json), None).await?;
    emit_wiki_progress(app, source_id, project_id, &source.filename, "done", 100, &format!("完成，生成 {} 个页面", written_pages.len()));

    Ok(IngestResult {
        task_id: task_id.to_string(),
        pages_created: written_pages.len(),
        page_paths: written_pages.iter().map(|p| p.path.clone()).collect(),
    })
}

/// Emit wiki source ingest progress event to frontend.
fn emit_wiki_progress(
    app: &AppHandle,
    source_id: &str,
    project_id: &str,
    filename: &str,
    stage: &str,
    progress: u8,
    detail: &str,
) {
    if let Err(error) = app.emit(
        "wiki-source-progress",
        serde_json::json!({
            "source_id": source_id,
            "project_id": project_id,
            "filename": filename,
            "stage": stage,
            "progress": progress,
            "detail": detail,
        }),
    ) {
        tracing::warn!(%error, source_id, project_id, "failed to emit Wiki ingest progress");
    }
}
