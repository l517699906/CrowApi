use super::processor::{self, DOCUMENT_TASK_ALREADY_RUNNING};
use super::repository::{KbRepository, KB_CONFIG_SUPERSEDED};
use super::retriever;
use crate::services::tasks::{
    emit_task_event,
    models::{BackgroundTask, TASK_CANCELLED},
    repository::TaskRepository,
};
use sqlx::SqlitePool;
use std::time::Duration;
use tauri::AppHandle;

fn parse_reprocess_payload(task: &BackgroundTask) -> Result<i64, String> {
    if task.domain != "knowledge"
        || task.task_type != "reprocess_knowledge_base"
        || task.resource_type != "knowledge_base"
    {
        return Err("后台任务不是知识库重处理任务".to_string());
    }
    let payload: serde_json::Value = serde_json::from_str(&task.payload_json)
        .map_err(|error| format!("知识库重处理任务参数损坏: {}", error))?;
    if payload
        .get("payload_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
        || payload.get("operation").and_then(serde_json::Value::as_str)
            != Some("reprocess_knowledge_base")
        || payload.get("kb_id").and_then(serde_json::Value::as_str)
            != Some(task.resource_id.as_str())
    {
        return Err("知识库重处理任务参数与资源不匹配".to_string());
    }
    payload
        .get("config_revision")
        .and_then(serde_json::Value::as_i64)
        .filter(|revision| *revision > 0)
        .ok_or_else(|| "知识库重处理任务缺少有效配置版本".to_string())
}

fn spawn_reprocess_task(
    pool: SqlitePool,
    app: AppHandle,
    task_id: String,
    kb_id: String,
    config_revision: i64,
) {
    tokio::spawn(async move {
        if let Err(error) = run_reprocess_task(
            &pool,
            &app,
            &task_id,
            &kb_id,
            config_revision,
        )
        .await
        {
            tracing::error!(
                %error,
                task_id,
                knowledge_base_id = %kb_id,
                config_revision,
                "knowledge base reprocessing failed"
            );
        }
    });
}

pub async fn start_existing_reprocess_task(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
) -> Result<(), String> {
    let tasks = TaskRepository::new(pool.clone());
    let task = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    if task.status != "pending" {
        return Err("知识库重处理任务已经开始或结束".to_string());
    }
    let config_revision = parse_reprocess_payload(&task)?;
    if !tasks
        .claim(task_id, "preparing")
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("知识库重处理任务已经开始或结束".to_string());
    }
    spawn_reprocess_task(
        pool.clone(),
        app.clone(),
        task.id,
        task.resource_id,
        config_revision,
    );
    Ok(())
}

async fn ensure_current_revision(
    repo: &KbRepository,
    tasks: &TaskRepository,
    task_id: &str,
    kb_id: &str,
    config_revision: i64,
) -> Result<(), String> {
    tasks.ensure_not_cancelled(task_id).await?;
    let current_revision = repo
        .get_kb(kb_id)
        .await
        .map_err(|error| error.to_string())?
        .config_revision;
    if current_revision == config_revision {
        Ok(())
    } else {
        Err(KB_CONFIG_SUPERSEDED.to_string())
    }
}

async fn run_reprocess_task(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
    kb_id: &str,
    config_revision: i64,
) -> Result<(), String> {
    let repo = KbRepository::new(pool.clone());
    let tasks = TaskRepository::new(pool.clone());
    let result = run_reprocess_task_inner(
        pool,
        app,
        task_id,
        kb_id,
        config_revision,
        &repo,
        &tasks,
    )
    .await;

    match &result {
        Ok(done_items) => {
            let result_json = serde_json::json!({
                "config_revision": config_revision,
                "processed_documents": done_items,
            })
            .to_string();
            tasks
                .succeed(task_id, Some(&result_json))
                .await
                .map_err(|error| error.to_string())?;
        }
        Err(error) if error == TASK_CANCELLED || error == KB_CONFIG_SUPERSEDED => {
            let _ = tasks.mark_cancelled(task_id).await;
        }
        Err(error) => {
            let _ = tasks.fail(task_id, error).await;
        }
    }
    if let Ok(task) = tasks.get(task_id).await {
        emit_task_event(app, &task, task.error_message.as_deref());
    }
    result.map(|_| ())
}

#[allow(clippy::too_many_arguments)]
async fn run_reprocess_task_inner(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
    kb_id: &str,
    config_revision: i64,
    repo: &KbRepository,
    tasks: &TaskRepository,
) -> Result<i64, String> {
    ensure_current_revision(repo, tasks, task_id, kb_id, config_revision).await?;
    let documents = repo
        .get_documents_needing_config_revision(kb_id, config_revision)
        .await
        .map_err(|error| error.to_string())?;
    let total_items = documents.len() as i64;
    tasks
        .update_progress(task_id, "reprocessing", 0, 0, total_items)
        .await
        .map_err(|error| error.to_string())?;
    if let Ok(task) = tasks.get(task_id).await {
        emit_task_event(app, &task, Some("按新配置重新处理知识库文档"));
    }

    let mut done_items = 0i64;
    let mut failures = Vec::new();
    for document in documents {
        ensure_current_revision(repo, tasks, task_id, kb_id, config_revision).await?;
        loop {
            match processor::reprocess_document_with_parent(
                pool,
                app,
                document.clone(),
                task_id,
            )
            .await
            {
                Ok(()) => break,
                Err(error) if error == DOCUMENT_TASK_ALREADY_RUNNING => {
                    ensure_current_revision(repo, tasks, task_id, kb_id, config_revision).await?;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(error) if error == TASK_CANCELLED || error == KB_CONFIG_SUPERSEDED => {
                    return Err(error);
                }
                Err(error) => {
                    let _ = repo
                        .update_document_status(&document.id, "stale", Some(&error))
                        .await;
                    failures.push(format!("{}: {}", document.filename, error));
                    break;
                }
            }
        }
        done_items += 1;
        let progress = if total_items == 0 {
            90
        } else {
            ((done_items * 90) / total_items).clamp(1, 90)
        };
        tasks
            .update_progress(
                task_id,
                "reprocessing",
                progress,
                done_items,
                total_items,
            )
            .await
            .map_err(|error| error.to_string())?;
        if let Ok(task) = tasks.get(task_id).await {
            emit_task_event(app, &task, Some(&format!("已处理 {}", document.filename)));
        }
    }

    ensure_current_revision(repo, tasks, task_id, kb_id, config_revision).await?;
    let remaining = repo
        .get_documents_needing_config_revision(kb_id, config_revision)
        .await
        .map_err(|error| error.to_string())?;
    if !failures.is_empty() || !remaining.is_empty() {
        let detail = failures.into_iter().take(5).collect::<Vec<_>>().join("; ");
        return Err(format!(
            "知识库仍有 {} 个文档未按当前配置完成处理{}{}",
            remaining.len(),
            if detail.is_empty() { "" } else { ": " },
            detail
        ));
    }

    tasks
        .update_progress(task_id, "indexing", 95, done_items, total_items)
        .await
        .map_err(|error| error.to_string())?;
    retriever::build_index_with_parent(pool, kb_id, app, task_id).await?;
    ensure_current_revision(repo, tasks, task_id, kb_id, config_revision).await?;
    Ok(done_items)
}

#[cfg(test)]
mod tests {
    use super::parse_reprocess_payload;
    use crate::services::tasks::models::BackgroundTask;

    fn task(payload_json: &str) -> BackgroundTask {
        BackgroundTask {
            id: "task-1".to_string(),
            domain: "knowledge".to_string(),
            task_type: "reprocess_knowledge_base".to_string(),
            resource_type: "knowledge_base".to_string(),
            resource_id: "kb-1".to_string(),
            subject_id: None,
            parent_task_id: None,
            idempotency_key: Some("knowledge:reprocess:kb-1".to_string()),
            retry_of: None,
            status: "pending".to_string(),
            stage: "queued".to_string(),
            progress: 0,
            total_items: 1,
            done_items: 0,
            payload_json: payload_json.to_string(),
            result_json: None,
            error_message: None,
            retryable: 1,
            auto_resume: 1,
            cancel_requested: 0,
            attempt: 1,
            created_at: "2026-08-21T00:00:00Z".to_string(),
            started_at: None,
            updated_at: "2026-08-21T00:00:00Z".to_string(),
            completed_at: None,
            lease_owner: None,
            lease_until: None,
            heartbeat_at: None,
            next_retry_at: None,
            max_attempts: 5,
            dead_letter: 0,
        }
    }

    #[test]
    fn validates_reprocess_payload_contract() {
        assert_eq!(
            parse_reprocess_payload(&task(
                r#"{"payload_version":1,"operation":"reprocess_knowledge_base","kb_id":"kb-1","config_revision":4}"#,
            ))
            .unwrap(),
            4
        );
        assert!(parse_reprocess_payload(&task(
            r#"{"payload_version":1,"operation":"reprocess_knowledge_base","kb_id":"other","config_revision":4}"#,
        ))
        .is_err());
    }
}
