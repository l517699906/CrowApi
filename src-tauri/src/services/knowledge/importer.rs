use super::parser;
use super::models::{ImportSourceInput, KbDocument, KbSource};
use super::processor;
use super::repository::KbRepository;
use super::retriever;
use super::storage;
use crate::services::tasks::{
    emit_task_event,
    models::{TaskSpec, TASK_CANCELLED},
    repository::TaskRepository,
};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use std::path::PathBuf;
use sha2::Digest;

pub const IMPORT_TASK_ALREADY_RUNNING: &str = "KB_IMPORT_TASK_ALREADY_RUNNING";

fn should_skip_import_document(
    existing_status: &str,
    existing_source_id: Option<&str>,
    source_id: &str,
) -> bool {
    existing_status == "ready" || existing_source_id != Some(source_id)
}

async fn remove_snapshot(path: &PathBuf) {
    if let Err(error) = tokio::fs::remove_file(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(%error, path = %path.display(), "failed to remove import snapshot");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn prepare_import_document(
    repo: &KbRepository,
    app: &AppHandle,
    kb_id: &str,
    source_id: &str,
    filename: &str,
    file_type: &str,
    content: &[u8],
    content_hash: &str,
    source_type: &str,
    source_url: Option<&str>,
    source_path: Option<&str>,
) -> Result<Option<KbDocument>, String> {
    let existing = repo
        .find_document_by_hash(kb_id, content_hash)
        .await
        .map_err(|error| format!("Failed to check duplicate document: {}", error))?;
    if let Some(existing) = existing.as_ref() {
        if should_skip_import_document(
            &existing.status,
            existing.source_id.as_deref(),
            source_id,
        ) {
            return Ok(None);
        }
    }

    let snapshot = storage::persist_import_snapshot(app, kb_id, content).await?;
    let snapshot_path = snapshot.to_string_lossy().to_string();
    if let Some(mut existing) = existing {
        let previous_path = existing.file_path.clone();
        if let Err(error) = repo
            .update_document_snapshot_path(kb_id, source_id, &existing.id, &snapshot_path)
            .await
        {
            remove_snapshot(&snapshot).await;
            return Err(format!("Failed to update import snapshot: {}", error));
        }
        existing.file_path = Some(snapshot_path);
        if let Some(previous_path) = previous_path.as_deref() {
            if previous_path != existing.file_path.as_deref().unwrap_or_default() {
                if let Err(error) = storage::remove_managed_document_file(
                    app,
                    kb_id,
                    std::path::Path::new(previous_path),
                )
                .await
                {
                    tracing::warn!(
                        %error,
                        document_id = %existing.id,
                        path = %previous_path,
                        "failed to remove superseded import snapshot"
                    );
                }
            }
        }
        return Ok(Some(existing));
    }

    match repo
        .create_document_with_source(
            kb_id,
            filename,
            Some(&snapshot_path),
            file_type,
            content.len() as i64,
            content_hash,
            Some(source_id),
            source_type,
            source_url,
            source_path,
        )
        .await
    {
        Ok(document) => Ok(Some(document)),
        Err(sqlx::Error::RowNotFound) => {
            remove_snapshot(&snapshot).await;
            Err("Import source no longer exists".to_string())
        }
        Err(error) => {
            remove_snapshot(&snapshot).await;
            Err(error.to_string())
        }
    }
}

pub async fn start_import_source(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    source: KbSource,
    input: ImportSourceInput,
) -> Result<String, String> {
    if source.kb_id != kb_id {
        return Err("知识来源不属于目标知识库".to_string());
    }
    let retryable = input.token.as_deref().is_none_or(str::is_empty);
    let spec = TaskSpec::new("knowledge", "import_source", "knowledge_base", kb_id)
        .subject_id(Some(source.id.clone()))
        .idempotency_key(format!("knowledge:source:{}", source.id))
        .payload(import_payload(&input))
        .retryable(retryable)
        .auto_resume(retryable);
    let task = TaskRepository::new(pool.clone())
        .create_if_idle(&spec)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| IMPORT_TASK_ALREADY_RUNNING.to_string())?;
    let task_id = task.id.clone();
    spawn_import_task(
        pool.clone(),
        app.clone(),
        source,
        input,
        task.id,
    );
    Ok(task_id)
}

pub async fn start_existing_import_task(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
) -> Result<(), String> {
    let task = TaskRepository::new(pool.clone())
        .get(task_id)
        .await
        .map_err(|error| error.to_string())?;
    if task.domain != "knowledge"
        || task.task_type != "import_source"
        || task.resource_type != "knowledge_base"
        || task.status != "pending"
    {
        return Err("后台任务不是可执行的知识来源导入任务".to_string());
    }
    let source_id = task
        .subject_id
        .as_deref()
        .ok_or_else(|| "知识来源导入任务缺少来源标识".to_string())?;
    let source = KbRepository::new(pool.clone())
        .get_source(source_id)
        .await
        .map_err(|error| error.to_string())?;
    if source.kb_id != task.resource_id {
        return Err("知识来源导入任务资源不匹配".to_string());
    }
    let payload: serde_json::Value = serde_json::from_str(&task.payload_json)
        .map_err(|error| format!("知识来源导入任务参数损坏: {}", error))?;
    if payload.get("payload_version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("知识来源导入任务参数版本不受支持".to_string());
    }
    let payload_source_type = payload
        .get("source_type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "知识来源导入任务缺少来源类型".to_string())?;
    if payload_source_type != source.source_type {
        return Err("知识来源导入任务来源类型不匹配".to_string());
    }
    let mut input: ImportSourceInput = serde_json::from_value(payload)
        .map_err(|error| format!("知识来源导入任务参数损坏: {}", error))?;
    input.token = None;
    input.source_type.clone_from(&source.source_type);
    input.branch.clone_from(&source.branch);
    match source.source_type.as_str() {
        "git" => input.repo_url.clone_from(&source.source_url),
        "url" => input.url.clone_from(&source.source_url),
        "local_dir" => input.dir_path.clone_from(&source.source_path),
        _ => return Err("知识来源类型不支持重试".to_string()),
    }
    spawn_import_task(
        pool.clone(),
        app.clone(),
        source,
        input,
        task.id,
    );
    Ok(())
}

fn import_payload(input: &ImportSourceInput) -> serde_json::Value {
    serde_json::json!({
        "payload_version": 1,
        "source_type": input.source_type,
        "repo_url": input.repo_url,
        "branch": input.branch,
        "url": input.url,
        "dir_path": input.dir_path,
        "excluded_dirs": input.excluded_dirs,
        "included_files": input.included_files,
        "max_file_size": input.max_file_size,
    })
}

fn spawn_import_task(
    pool: SqlitePool,
    app: AppHandle,
    source: KbSource,
    input: ImportSourceInput,
    task_id: String,
) {
    tokio::spawn(async move {
        if let Err(error) = run_import_task(&pool, &app, source, input, &task_id).await {
            tracing::error!(%error, %task_id, "knowledge source import failed");
        }
    });
}

async fn run_import_task(
    pool: &SqlitePool,
    app: &AppHandle,
    source: KbSource,
    input: ImportSourceInput,
    task_id: &str,
) -> Result<(), String> {
    let tasks = TaskRepository::new(pool.clone());
    if !tasks
        .claim(task_id, "preparing")
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("知识来源导入任务已经开始或结束".to_string());
    }
    let repo = KbRepository::new(pool.clone());
    let result = async {
        repo.update_source_status(&source.id, "running", source.file_count, None)
            .await
            .map_err(|error| error.to_string())?;
        emit_import_progress(
            pool,
            app,
            task_id,
            &source.kb_id,
            &source.id,
            "preparing",
            0,
            0,
            0,
            "准备导入知识来源",
        )
        .await?;

        let count = match input.source_type.as_str() {
            "git" => import_git_repo(pool, app, &source.kb_id, &source.id, task_id, &input).await,
            "url" => import_url(pool, app, &source.kb_id, &source.id, task_id, &input).await,
            "local_dir" => {
                import_local_dir(pool, app, &source.kb_id, &source.id, task_id, &input).await
            }
            _ => Err(format!("Unknown source type: {}", input.source_type)),
        }?;
        tasks.ensure_not_cancelled(task_id).await?;
        Ok::<usize, String>(count)
    }
    .await;

    if result.is_ok() {
        if let Err(error) = retriever::schedule_index_build(pool, &source.kb_id, app).await {
            tracing::warn!(%error, knowledge_base_id = %source.kb_id, "failed to schedule index after source import");
        }
    }

    let terminal_result = match result {
        Ok(count) => {
            let completion = async {
                let total_count = repo
                    .count_documents_by_source(&source.id)
                    .await
                    .map_err(|error| error.to_string())?;
                repo.update_source_status(&source.id, "done", total_count, None)
                    .await
                    .map_err(|error| error.to_string())?;
                let result_json = serde_json::json!({
                    "processed_this_attempt": count,
                    "total_documents": total_count,
                })
                .to_string();
                tasks
                    .succeed(task_id, Some(&result_json))
                    .await
                    .map_err(|error| error.to_string())
            }
            .await;
            match completion {
                Ok(()) => Ok(()),
                Err(error) => {
                    let message = format!("Failed to finalize knowledge source import: {}", error);
                    Err(fail_import_task(&repo, &tasks, &source, task_id, &message).await)
                }
            }
        }
        Err(error) if error == TASK_CANCELLED => {
            let persisted_count = repo
                .count_documents_by_source(&source.id)
                .await
                .unwrap_or(source.file_count);
            if let Err(status_error) = repo
                .update_source_status(&source.id, "cancelled", persisted_count, None)
                .await
            {
                tracing::warn!(%status_error, source_id = %source.id, "failed to mark cancelled source import");
            }
            if let Ok(task) = tasks.get(task_id).await {
                if matches!(task.status.as_str(), "pending" | "running") {
                    let _ = tasks.mark_cancelled(task_id).await;
                }
            }
            Ok(())
        }
        Err(error) => Err(fail_import_task(&repo, &tasks, &source, task_id, &error).await),
    };
    if let Ok(task) = tasks.get(task_id).await {
        emit_task_event(app, &task, task.error_message.as_deref());
    }
    terminal_result
}

async fn fail_import_task(
    repo: &KbRepository,
    tasks: &TaskRepository,
    source: &KbSource,
    task_id: &str,
    error: &str,
) -> String {
    let persisted_count = repo.count_documents_by_source(&source.id).await.unwrap_or(0);
    let mut finalization_errors = Vec::new();
    if let Err(status_error) = repo
        .update_source_status(&source.id, "error", persisted_count, Some(error))
        .await
    {
        finalization_errors.push(format!("source status: {}", status_error));
    }
    match tasks.get(task_id).await {
        Ok(task) if matches!(task.status.as_str(), "pending" | "running") => {
            if let Err(task_error) = tasks.fail(task_id, error).await {
                finalization_errors.push(format!("task status: {}", task_error));
            }
        }
        Ok(_) => {}
        Err(task_error) => finalization_errors.push(format!("task lookup: {}", task_error)),
    }
    if finalization_errors.is_empty() {
        error.to_string()
    } else {
        format!("{}; finalization failed: {}", error, finalization_errors.join(", "))
    }
}

/// Import a Git repository: clone → filter → process files
pub async fn import_git_repo(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    source_id: &str,
    task_id: &str,
    input: &ImportSourceInput,
) -> Result<usize, String> {
    let repo_url = input.repo_url.as_ref()
        .ok_or("repo_url is required for git import")?;

    let branch = input.branch.as_deref().unwrap_or("main");
    let token = input.token.as_deref().filter(|token| !token.is_empty());

    // Clone repo to temp dir. Credentials stay out of the URL and argv.
    let temp_dir = std::env::temp_dir().join(format!("kb_import_{}", uuid::Uuid::new_v4()));

    emit_import_progress(
        pool, app, task_id, kb_id, source_id, "cloning", 5, 0, 0,
        "正在克隆 Git 仓库",
    )
    .await?;

    let mut command = tokio::process::Command::new("git");
    command
        .args(["clone", "--depth", "1", "--branch", branch, repo_url])
        .arg(&temp_dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(token) = token {
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env("GIT_CONFIG_VALUE_0", format!("Authorization: Bearer {}", token));
    }
    let clone_result = command
        .output()
        .await
        .map_err(|e| format!("Failed to run git clone: {}", e))?;

    TaskRepository::new(pool.clone())
        .ensure_not_cancelled(task_id)
        .await?;
    if !clone_result.status.success() {
        let mut err = String::from_utf8_lossy(&clone_result.stderr).to_string();
        if let Some(token) = token {
            err = err.replace(token, "[REDACTED]");
        }
        // Clean up
        std::fs::remove_dir_all(&temp_dir).ok();
        return Err(format!("Git clone failed: {}", err.chars().take(500).collect::<String>()));
    }

    // Process files from cloned repo
    let excluded_dirs = input.excluded_dirs.clone().unwrap_or_default();
    let included_files = input.included_files.clone().unwrap_or_default();
    let max_file_size = input.max_file_size.unwrap_or(1024 * 1024); // 1MB default

    let result = process_directory_files(
        pool, app, kb_id, source_id, task_id, &temp_dir,
        &excluded_dirs, &included_files, max_file_size,
        "git", Some(repo_url), None,
    ).await;

    // Clean up temp dir
    std::fs::remove_dir_all(&temp_dir).ok();

    result
}

/// Import from a URL: fetch content → process
pub async fn import_url(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    source_id: &str,
    task_id: &str,
    input: &ImportSourceInput,
) -> Result<usize, String> {
    let url = input.url.as_ref().ok_or("url is required for url import")?;

    emit_import_progress(
        pool, app, task_id, kb_id, source_id, "fetching", 10, 0, 1,
        "正在读取 URL 内容",
    )
    .await?;

    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch URL: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), url));
    }

    TaskRepository::new(pool.clone())
        .ensure_not_cancelled(task_id)
        .await?;
    let content_type = resp.headers().get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .to_string();

    let content = resp.bytes().await.map_err(|e| format!("Failed to read body: {}", e))?;

    let parsed_url = reqwest::Url::parse(url)
        .map_err(|error| format!("Invalid import URL: {}", error))?;
    let filename = parsed_url
        .path_segments()
        .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
        .unwrap_or("imported")
        .to_string();
    let filename = if filename.contains('.') {
        filename
    } else if content_type.contains("html") {
        format!("{}.html", filename)
    } else if content_type.contains("markdown") || content_type.contains("text/plain") {
        format!("{}.md", filename)
    } else {
        format!("{}.txt", filename)
    };

    let file_type = parser::get_file_type(&filename);

    let repo = KbRepository::new(pool.clone());
    let hash = sha2::Sha256::digest(&content);
    let hash_hex = hex::encode(hash);
    let Some(doc) = prepare_import_document(
        &repo,
        app,
        kb_id,
        source_id,
        &filename,
        &file_type,
        &content,
        &hash_hex,
        "url",
        Some(url),
        None,
    )
    .await?
    else {
        emit_import_progress(
            pool, app, task_id, kb_id, source_id, "completed", 100, 1, 1,
            "URL 内容已存在，已跳过",
        )
        .await?;
        return Ok(0);
    };

    // Get KB embedding model
    let kb = repo.get_kb(kb_id).await.map_err(|e| e.to_string())?;
    let emb_model = kb.embedding_model.clone();

    emit_import_progress(
        pool, app, task_id, kb_id, source_id, "processing", 30, 0, 1,
        "正在处理 URL 文档",
    )
    .await?;

    processor::process_document_with_parent(
        pool,
        app,
        kb_id,
        &doc.id,
        &filename,
        &content,
        emb_model.as_deref(),
        Some(task_id),
        false,
    ).await?;

    emit_import_progress(
        pool, app, task_id, kb_id, source_id, "completed", 100, 1, 1,
        "URL 导入完成",
    )
    .await?;
    Ok(1)
}

/// Import from a local directory: scan → filter → process files
pub async fn import_local_dir(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    source_id: &str,
    task_id: &str,
    input: &ImportSourceInput,
) -> Result<usize, String> {
    let dir_path = input.dir_path.as_ref()
        .ok_or("dir_path is required for local_dir import")?;

    let path = PathBuf::from(dir_path);
    if !path.is_dir() {
        return Err(format!("Directory not found: {}", dir_path));
    }

    let excluded_dirs = input.excluded_dirs.clone().unwrap_or_default();
    let included_files = input.included_files.clone().unwrap_or_default();
    let max_file_size = input.max_file_size.unwrap_or(1024 * 1024);

    process_directory_files(
        pool, app, kb_id, source_id, task_id, &path,
        &excluded_dirs, &included_files, max_file_size,
        "local_dir", None, Some(dir_path),
    ).await
}

/// Common: process all files in a directory with filtering
async fn process_directory_files(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    source_id: &str,
    task_id: &str,
    dir: &PathBuf,
    excluded_dirs: &[String],
    included_files: &[String],
    max_file_size: usize,
    source_type: &str,
    source_url: Option<&str>,
    _source_path: Option<&str>,
) -> Result<usize, String> {
    emit_import_progress(
        pool, app, task_id, kb_id, source_id, "scanning", 10, 0, 0,
        "正在扫描目录",
    )
    .await?;

    let files = scan_directory(dir, excluded_dirs, included_files, max_file_size)?;

    if files.is_empty() {
        emit_import_progress(
            pool, app, task_id, kb_id, source_id, "completed", 100, 0, 0,
            "没有找到符合条件的文件",
        )
        .await?;
        return Ok(0);
    }

    let total = files.len();
    let repo = KbRepository::new(pool.clone());
    let kb = repo.get_kb(kb_id).await.map_err(|e| e.to_string())?;
    let emb_model = kb.embedding_model.clone();

    let mut processed = 0usize;
    let mut skipped = 0usize;

    for (i, file_path) in files.iter().enumerate() {
        TaskRepository::new(pool.clone())
            .ensure_not_cancelled(task_id)
            .await?;
        let filename = file_path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("file_{}", i));

        let pct = 10 + ((i as f64 / total as f64) * 80.0) as u8;
        emit_import_progress(
            pool,
            app,
            task_id,
            kb_id,
            source_id,
            "processing",
            pct,
            i as i64,
            total as i64,
            &format!("正在处理 {}/{}：{}", i + 1, total, filename),
        )
        .await?;

        // Read file
        let content = match std::fs::read(file_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to read file {}: {}", filename, e);
                skipped += 1;
                continue;
            }
        };

        // Hash
        let hash = sha2::Sha256::digest(&content);
        let hash_hex = hex::encode(hash);

        let file_type = parser::get_file_type(&filename);
        let rel_path = file_path.strip_prefix(dir)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        let doc = match prepare_import_document(
            &repo,
            app,
            kb_id,
            source_id,
            &filename,
            &file_type,
            &content,
            &hash_hex,
            source_type,
            source_url,
            Some(&rel_path),
        )
        .await
        {
            Ok(Some(document)) => document,
            Ok(None) => {
                skipped += 1;
                continue;
            }
            Err(error) => {
                if error == "Import source no longer exists" {
                    return Err(error);
                }
                tracing::warn!(%error, %filename, "failed to prepare imported document");
                skipped += 1;
                continue;
            }
        };

        // Process document
        if let Err(e) = processor::process_document_with_parent(
            pool,
            app,
            kb_id,
            &doc.id,
            &filename,
            &content,
            emb_model.as_deref(),
            Some(task_id),
            false,
        ).await {
            tracing::warn!("Failed to process document {}: {}", filename, e);
            skipped += 1;
        } else {
            processed += 1;
        }
    }

    // Update KB counts
    repo.update_kb_counts(kb_id)
        .await
        .map_err(|error| format!("Failed to update knowledge base counts: {}", error))?;

    emit_import_progress(
        pool,
        app,
        task_id,
        kb_id,
        source_id,
        "completed",
        100,
        total as i64,
        total as i64,
        &format!("导入完成：{} 个成功，{} 个跳过", processed, skipped),
    )
    .await?;
    Ok(processed)
}

/// Recursively scan directory, applying filters
fn scan_directory(
    dir: &PathBuf,
    excluded_dirs: &[String],
    included_files: &[String],
    max_file_size: usize,
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();

    // Default excluded dirs
    let default_excluded = vec![
        ".git", ".svn", ".hg", "node_modules", "__pycache__",
        ".venv", "venv", "env", ".env", "dist", "build",
        "target", ".next", ".nuxt", ".output", "vendor",
        "vendor", ".idea", ".vscode",
    ];

    let mut all_excluded: Vec<&str> = default_excluded.iter().copied().collect();
    for d in excluded_dirs {
        all_excluded.push(d.as_str());
    }

    scan_directory_recursive(dir, &all_excluded, included_files, max_file_size, &mut files)?;

    files.sort();
    Ok(files)
}

fn scan_directory_recursive(
    dir: &PathBuf,
    excluded_dirs: &[&str],
    included_files: &[String],
    max_file_size: usize,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Failed to read dir: {}", e))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            // Check if excluded
            if excluded_dirs.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            scan_directory_recursive(&path, excluded_dirs, included_files, max_file_size, files)?;
        } else if path.is_file() {
            // Check file size
            if let Ok(meta) = entry.metadata() {
                if meta.len() as usize > max_file_size {
                    continue;
                }
            }

            // Check extension
            let ext = path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            // Supported extensions
            let supported = is_supported_extension(&ext);

            // If included_files is specified, only include matching files
            let included = if included_files.is_empty() {
                true
            } else {
                included_files.iter().any(|f| {
                    let f_lower = f.to_lowercase();
                    name.to_lowercase().contains(&f_lower) || f_lower == ext
                })
            };

            if supported && included {
                files.push(path);
            }
        }
    }

    Ok(())
}

fn is_supported_extension(ext: &str) -> bool {
    matches!(ext,
        "md" | "markdown" |
        "txt" | "rst" | "log" |
        "rs" | "go" | "py" | "ts" | "tsx" | "js" | "jsx" |
        "java" | "c" | "cpp" | "h" | "hpp" | "cs" | "php" |
        "swift" | "kt" | "rb" | "scala" | "clj" | "sh" | "bash" |
        "vue" | "svelte" | "sql" | "proto" | "gradle" |
        "json" | "yaml" | "yml" | "toml" | "xml" | "html" | "csv" |
        "env" | "ini" | "conf" | "cfg" | "svg" |
        "pdf"
    )
}

async fn emit_import_progress(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
    kb_id: &str,
    source_id: &str,
    stage: &str,
    progress: u8,
    done_items: i64,
    total_items: i64,
    detail: &str,
) -> Result<(), String> {
    let tasks = TaskRepository::new(pool.clone());
    tasks.ensure_not_cancelled(task_id).await?;
    tasks
        .update_progress(
            task_id,
            stage,
            i64::from(progress),
            done_items,
            total_items,
        )
        .await
        .map_err(|error| error.to_string())?;
    if let Ok(task) = tasks.get(task_id).await {
        emit_task_event(app, &task, Some(detail));
    }
    let _ = app.emit("kb-import-progress", serde_json::json!({
        "task_id": task_id,
        "kb_id": kb_id,
        "source_id": source_id,
        "progress": progress,
        "detail": detail,
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{import_payload, should_skip_import_document, ImportSourceInput};

    #[test]
    fn import_task_payload_never_contains_git_token() {
        let payload = import_payload(&ImportSourceInput {
            source_type: "git".to_string(),
            repo_url: Some("https://example.com/repo.git".to_string()),
            branch: Some("main".to_string()),
            token: Some("super-secret-token".to_string()),
            url: None,
            dir_path: None,
            excluded_dirs: Some(vec!["target".to_string()]),
            included_files: None,
            max_file_size: Some(1024),
        });
        let serialized = payload.to_string();
        assert!(!serialized.contains("super-secret-token"));
        assert!(payload.get("token").is_none());
        assert_eq!(payload.get("payload_version").and_then(|value| value.as_u64()), Some(1));
    }

    #[test]
    fn completed_imports_are_skipped_and_incomplete_same_source_documents_are_reused() {
        assert!(should_skip_import_document("ready", Some("source-1"), "source-1"));
        assert!(!should_skip_import_document(
            "processing",
            Some("source-1"),
            "source-1"
        ));
        assert!(!should_skip_import_document("failed", Some("source-1"), "source-1"));
        assert!(should_skip_import_document(
            "processing",
            Some("source-2"),
            "source-1"
        ));
    }
}
