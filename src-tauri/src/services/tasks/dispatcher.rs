use super::{
    emit_task_event,
    models::BackgroundTask,
    repository::TaskRepository,
};
use sqlx::SqlitePool;
use tauri::AppHandle;
use chrono::{Duration as ChronoDuration, Utc};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoverySummary {
    pub eligible: usize,
    pub resumed: usize,
    pub failed: usize,
}

pub fn spawn_maintenance(pool: SqlitePool, app: AppHandle) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        let mut last_retention = Instant::now() - Duration::from_secs(3_600);
        loop {
            interval.tick().await;
            let tasks = TaskRepository::new(pool.clone());
            match tasks.reap_expired_leases().await {
                Ok(expired) if expired > 0 => {
                    tracing::warn!(expired, "reaped expired background task leases");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "failed to reap expired background task leases");
                    continue;
                }
            }
            let due = match tasks.list_auto_resumable_interrupted().await {
                Ok(tasks) => tasks,
                Err(error) => {
                    tracing::warn!(%error, "failed to inspect scheduled background task retries");
                    continue;
                }
            };
            for task in due {
                if let Err(error) = retry_and_dispatch(&pool, &app, &task.id).await {
                    tracing::warn!(
                        %error,
                        task_id = %task.id,
                        domain = %task.domain,
                        task_type = %task.task_type,
                        "failed to dispatch scheduled background task retry"
                    );
                }
            }
            if last_retention.elapsed() >= Duration::from_secs(3_600) {
                run_retention_maintenance(&pool, &app).await;
                last_retention = Instant::now();
            }
        }
    });
}

async fn run_retention_maintenance(pool: &SqlitePool, app: &AppHandle) {
    let settings = crate::config::load_settings(app);
    let now = Utc::now();
    if settings.log_retention_days > 0 {
        let cutoff = (now - ChronoDuration::days(settings.log_retention_days)).to_rfc3339();
        match crate::db::repository::Repository::new(pool.clone())
            .purge_logs_before(&cutoff, 500)
            .await
        {
            Ok(deleted) if deleted > 0 => tracing::info!(deleted, "自动清理过期请求日志"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "自动清理请求日志失败"),
        }
    }
    if settings.task_retention_days > 0 {
        let cutoff = (now - ChronoDuration::days(settings.task_retention_days)).to_rfc3339();
        match TaskRepository::new(pool.clone())
            .purge_finished_before(&cutoff, 500)
            .await
        {
            Ok(deleted) if deleted > 0 => tracing::info!(deleted, "自动清理过期后台任务"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "自动清理后台任务失败"),
        }
    }
}

pub async fn recover_interrupted(
    pool: &SqlitePool,
    app: &AppHandle,
) -> Result<RecoverySummary, String> {
    let tasks = TaskRepository::new(pool.clone());
    let interrupted = tasks
        .list_auto_resumable_interrupted()
        .await
        .map_err(|error| error.to_string())?;
    let mut summary = RecoverySummary {
        eligible: interrupted.len(),
        ..RecoverySummary::default()
    };

    for task in interrupted {
        match retry_and_dispatch(pool, app, &task.id).await {
            Ok(retried) => {
                summary.resumed += 1;
                tracing::info!(
                    original_task_id = %task.id,
                    resumed_task_id = %retried.id,
                    domain = %task.domain,
                    task_type = %task.task_type,
                    "automatically resumed interrupted background task"
                );
            }
            Err(error) => {
                summary.failed += 1;
                tracing::warn!(
                    %error,
                    task_id = %task.id,
                    domain = %task.domain,
                    task_type = %task.task_type,
                    "failed to automatically resume interrupted background task"
                );
            }
        }
    }

    Ok(summary)
}

pub async fn retry_and_dispatch(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
) -> Result<BackgroundTask, String> {
    let tasks = TaskRepository::new(pool.clone());
    let original = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    ensure_supported(&original)?;
    let retried = tasks
        .retry(task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "任务不可重试，或同类任务已经在运行".to_string())?;
    dispatch_existing(pool, app, &retried.id).await
}

pub async fn dispatch_existing(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
) -> Result<BackgroundTask, String> {
    let tasks = TaskRepository::new(pool.clone());
    let task = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    ensure_supported(&task)?;
    let result = match (task.domain.as_str(), task.task_type.as_str()) {
        ("knowledge", "process_document" | "reindex_document") => {
            crate::services::knowledge::processor::start_existing_document_task(
                pool, app, task_id,
            )
            .await
        }
        ("knowledge", "build_index") => {
            crate::services::knowledge::retriever::start_existing_index_task(
                pool, app, task_id,
            )
            .await
        }
        ("knowledge", "reprocess_knowledge_base") => {
            crate::services::knowledge::reprocessor::start_existing_reprocess_task(
                pool, app, task_id,
            )
            .await
        }
        ("knowledge", "import_source") => {
            crate::services::knowledge::importer::start_existing_import_task(
                pool, app, task_id,
            )
            .await
        }
        ("wiki", "ingest") => {
            crate::services::wiki::ingest::start_existing_ingest_task(app, pool, task_id).await
        }
        ("maintenance", "cleanup_staged_path") => {
            crate::services::knowledge::lifecycle::start_existing_cleanup_task(
                pool, app, task_id,
            )
            .await
        }
        _ => unreachable!("supported task mapping must be exhaustive"),
    };
    if let Err(error) = result {
        if let Ok(current) = tasks.get(task_id).await {
            if current.status == "pending" {
                let _ = tasks.fail(task_id, &error).await;
            }
        }
        if let Ok(failed) = tasks.get(task_id).await {
            emit_task_event(app, &failed, Some(&error));
        }
        return Err(error);
    }
    let running = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    emit_task_event(app, &running, Some("任务已重新启动"));
    Ok(running)
}

fn ensure_supported(task: &BackgroundTask) -> Result<(), String> {
    if matches!(
        (task.domain.as_str(), task.task_type.as_str()),
        ("knowledge", "process_document")
            | ("knowledge", "reindex_document")
            | ("knowledge", "build_index")
            | ("knowledge", "reprocess_knowledge_base")
            | ("knowledge", "import_source")
            | ("wiki", "ingest")
            | ("maintenance", "cleanup_staged_path")
    ) {
        Ok(())
    } else {
        Err(format!(
            "不支持重试后台任务 {}:{}",
            task.domain, task.task_type
        ))
    }
}
