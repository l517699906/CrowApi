use super::models::{BackgroundTask, TaskListFilter, TaskSpec, TASK_CANCELLED};
use crate::db::models::now_iso;
use chrono::{Duration as ChronoDuration, Utc};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use std::sync::OnceLock;

const LEASE_SECONDS: i64 = 300;

fn worker_owner() -> &'static str {
    static OWNER: OnceLock<String> = OnceLock::new();
    OWNER.get_or_init(|| format!("crowapi-worker-{}", uuid::Uuid::new_v4()))
}

fn lease_until() -> String {
    (Utc::now() + ChronoDuration::seconds(LEASE_SECONDS)).to_rfc3339()
}

fn next_retry_at(attempt: i64, max_attempts: i64) -> Option<String> {
    if attempt >= max_attempts {
        return None;
    }
    let exponent = attempt.saturating_sub(1).clamp(0, 6) as u32;
    let delay_seconds = (5_i64.saturating_mul(1_i64 << exponent)).min(300);
    Some((Utc::now() + ChronoDuration::seconds(delay_seconds)).to_rfc3339())
}

#[derive(Clone)]
pub struct TaskRepository {
    pool: SqlitePool,
}

impl TaskRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_if_idle(
        &self,
        spec: &TaskSpec,
    ) -> Result<Option<BackgroundTask>, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        let payload_json = serde_json::to_string(&spec.payload)
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        let result = sqlx::query(
            "INSERT INTO background_tasks
             (id, domain, task_type, resource_type, resource_id, subject_id,
              parent_task_id, idempotency_key, retry_of, status, stage, progress, total_items,
              done_items, payload_json, retryable, auto_resume, cancel_requested, attempt,
              created_at, updated_at, lease_owner, lease_until, heartbeat_at, next_retry_at,
              max_attempts, dead_letter)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', 'queued', 0, ?, 0, ?, ?, ?, 0, ?, ?, ?, NULL, NULL, NULL, NULL, ?, 0)
             ON CONFLICT DO NOTHING",
        )
        .bind(&id)
        .bind(&spec.domain)
        .bind(&spec.task_type)
        .bind(&spec.resource_type)
        .bind(&spec.resource_id)
        .bind(&spec.subject_id)
        .bind(&spec.parent_task_id)
        .bind(&spec.idempotency_key)
        .bind(&spec.retry_of)
        .bind(spec.total_items)
        .bind(payload_json)
        .bind(i64::from(spec.retryable))
        .bind(i64::from(spec.auto_resume))
        .bind(spec.attempt.max(1))
        .bind(&now)
        .bind(&now)
        .bind(spec.max_attempts.max(1))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get(&id).await.map(Some)
    }

    pub async fn get(&self, id: &str) -> Result<BackgroundTask, sqlx::Error> {
        sqlx::query_as::<_, BackgroundTask>("SELECT * FROM background_tasks WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn list(
        &self,
        filter: &TaskListFilter,
    ) -> Result<Vec<BackgroundTask>, sqlx::Error> {
        let mut query = QueryBuilder::<Sqlite>::new("SELECT * FROM background_tasks WHERE 1 = 1");
        if let Some(domain) = filter.domain.as_deref() {
            query.push(" AND domain = ").push_bind(domain);
        }
        if let Some(resource_type) = filter.resource_type.as_deref() {
            query.push(" AND resource_type = ").push_bind(resource_type);
        }
        if let Some(resource_id) = filter.resource_id.as_deref() {
            query.push(" AND resource_id = ").push_bind(resource_id);
        }
        if let Some(status) = filter.status.as_deref() {
            query.push(" AND status = ").push_bind(status);
        }
        query
            .push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(filter.limit.unwrap_or(50).clamp(1, 200));
        query.build_query_as::<BackgroundTask>().fetch_all(&self.pool).await
    }

    pub async fn claim(&self, id: &str, stage: &str) -> Result<bool, sqlx::Error> {
        let now = now_iso();
        let lease_until = lease_until();
        let result = sqlx::query(
            "UPDATE background_tasks
             SET status = 'running', stage = ?, started_at = COALESCE(started_at, ?),
                 updated_at = ?, lease_owner = ?, lease_until = ?, heartbeat_at = ?
             WHERE id = ? AND status = 'pending' AND cancel_requested = 0
               AND (next_retry_at IS NULL OR next_retry_at <= ?)",
        )
        .bind(stage)
        .bind(&now)
        .bind(&now)
        .bind(worker_owner())
        .bind(&lease_until)
        .bind(&now)
        .bind(id)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        let claimed = result.rows_affected() == 1;
        if claimed {
            self.spawn_heartbeat(id.to_string());
        }
        Ok(claimed)
    }

    fn spawn_heartbeat(&self, task_id: String) {
        let repository = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.tick().await;
            loop {
                interval.tick().await;
                if repository.heartbeat(&task_id).await.is_err() {
                    break;
                }
            }
        });
    }

    pub async fn heartbeat(&self, id: &str) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let lease_until = lease_until();
        let result = sqlx::query(
            "UPDATE background_tasks
             SET lease_until = ?, heartbeat_at = ?, updated_at = ?
             WHERE id = ? AND status = 'running' AND lease_owner = ?",
        )
        .bind(&lease_until)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .bind(worker_owner())
        .execute(&self.pool)
        .await?;
        require_task_row(result.rows_affected())
    }

    pub async fn update_progress(
        &self,
        id: &str,
        stage: &str,
        progress: i64,
        done_items: i64,
        total_items: i64,
    ) -> Result<(), sqlx::Error> {
        let result = sqlx::query(
            "UPDATE background_tasks
             SET stage = ?, progress = ?, done_items = ?, total_items = ?, updated_at = ?,
                 lease_until = ?, heartbeat_at = ?
             WHERE id = ? AND status = 'running' AND lease_owner = ?",
        )
        .bind(stage)
        .bind(progress.clamp(0, 100))
        .bind(done_items.max(0))
        .bind(total_items.max(0))
        .bind(now_iso())
        .bind(lease_until())
        .bind(now_iso())
        .bind(id)
        .bind(worker_owner())
        .execute(&self.pool)
        .await?;
        require_task_row(result.rows_affected())
    }

    pub async fn succeed(
        &self,
        id: &str,
        result_json: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        self.finish(id, "succeeded", "completed", result_json, None)
            .await
    }

    pub async fn fail(&self, id: &str, error: &str) -> Result<(), sqlx::Error> {
        self.finish(id, "failed", "failed", None, Some(error)).await
    }

    pub async fn mark_cancelled(&self, id: &str) -> Result<(), sqlx::Error> {
        self.finish(id, "cancelled", "cancelled", None, None).await
    }

    async fn finish(
        &self,
        id: &str,
        status: &str,
        stage: &str,
        result_json: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let progress = if status == "succeeded" { 100 } else { 0 };
        let current = self.get(id).await?;
        let dead_letter = i64::from(
            status == "failed" && current.retryable == 1 && current.attempt >= current.max_attempts,
        );
        let retry_at = if status == "failed" && current.retryable == 1 && dead_letter == 0 {
            next_retry_at(current.attempt, current.max_attempts)
        } else {
            None
        };
        let result = sqlx::query(
            "UPDATE background_tasks
             SET status = CASE WHEN cancel_requested = 1 THEN 'cancelled' ELSE ? END,
                 stage = CASE WHEN cancel_requested = 1 THEN 'cancelled' ELSE ? END,
                 progress = CASE WHEN cancel_requested = 0 AND ? = 100 THEN 100 ELSE progress END,
                 result_json = CASE WHEN cancel_requested = 1 THEN NULL ELSE ? END,
                 error_message = CASE WHEN cancel_requested = 1 THEN NULL ELSE ? END,
                 updated_at = ?, completed_at = ?,
                 lease_owner = NULL, lease_until = NULL, heartbeat_at = NULL,
                 next_retry_at = CASE WHEN cancel_requested = 1 THEN NULL ELSE ? END,
                 dead_letter = CASE WHEN cancel_requested = 1 THEN 0 ELSE ? END
             WHERE id = ? AND status IN ('pending', 'running')",
        )
        .bind(status)
        .bind(stage)
        .bind(progress)
        .bind(result_json)
        .bind(error)
        .bind(&now)
        .bind(&now)
        .bind(retry_at)
        .bind(dead_letter)
        .bind(id)
        .execute(&self.pool)
        .await?;
        require_task_row(result.rows_affected())
    }

    pub async fn request_cancel(&self, id: &str) -> Result<BackgroundTask, sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query(
            "UPDATE background_tasks
             SET cancel_requested = 1,
                 status = CASE WHEN status = 'pending' THEN 'cancelled' ELSE status END,
                 stage = CASE WHEN status = 'pending' THEN 'cancelled' ELSE stage END,
                 completed_at = CASE WHEN status = 'pending' THEN ? ELSE completed_at END,
                 lease_owner = NULL,
                 lease_until = NULL,
                 heartbeat_at = ?,
                 updated_at = ?
             WHERE id = ? AND status IN ('pending', 'running')",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        require_task_row(result.rows_affected())?;
        self.get(id).await
    }

    pub async fn ensure_not_cancelled(&self, id: &str) -> Result<(), String> {
        let cancel_requested: i64 = sqlx::query_scalar(
            "SELECT cancel_requested FROM background_tasks WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        if cancel_requested == 0 {
            let _ = self.heartbeat(id).await;
            return Ok(());
        }
        let _ = self.mark_cancelled(id).await;
        Err(TASK_CANCELLED.to_string())
    }

    pub async fn retry(&self, id: &str) -> Result<Option<BackgroundTask>, sqlx::Error> {
        let original = self.get(id).await?;
        if original.retryable == 0
            || original.dead_letter == 1
            || original.attempt >= original.max_attempts
            || !matches!(
                original.status.as_str(),
                "failed" | "cancelled" | "interrupted"
            )
        {
            return Ok(None);
        }
        let payload = serde_json::from_str(&original.payload_json)
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        let original_id = original.id.clone();
        let mut spec = TaskSpec::new(
            original.domain,
            original.task_type,
            original.resource_type,
            original.resource_id,
        )
        .subject_id(original.subject_id)
        .parent_task_id(original.parent_task_id)
        .payload(payload)
        .retryable(true)
        .auto_resume(original.auto_resume == 1)
        .total_items(original.total_items)
        .max_attempts(original.max_attempts);
        spec.idempotency_key = original.idempotency_key;
        spec.retry_of = Some(original_id.clone());
        spec.attempt = original.attempt + 1;
        let retried = self.create_if_idle(&spec).await?;
        if retried.is_some() {
            sqlx::query(
                "UPDATE background_tasks
                 SET auto_resume = 0, next_retry_at = NULL, updated_at = ?
                 WHERE id = ?",
            )
            .bind(now_iso())
            .bind(original_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(retried)
    }

    pub async fn interrupt_inflight(&self) -> Result<u64, sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query(
            "UPDATE background_tasks
             SET status = 'interrupted', stage = 'interrupted',
                 error_message = COALESCE(error_message, '应用在任务完成前退出'),
                 updated_at = ?, completed_at = ?,
                 lease_owner = NULL, lease_until = NULL, heartbeat_at = NULL
             WHERE status IN ('pending', 'running')",
        )
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn reap_expired_leases(&self) -> Result<u64, sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query(
            "UPDATE background_tasks
             SET status = 'interrupted', stage = 'interrupted',
                 error_message = COALESCE(error_message, '后台任务租约已过期'),
                 updated_at = ?, completed_at = ?,
                 lease_owner = NULL, lease_until = NULL, heartbeat_at = NULL
             WHERE status = 'running' AND lease_until IS NOT NULL AND lease_until < ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_auto_resumable_interrupted(
        &self,
    ) -> Result<Vec<BackgroundTask>, sqlx::Error> {
        let now = now_iso();
        sqlx::query_as::<_, BackgroundTask>(
            "SELECT * FROM background_tasks
             WHERE (status = 'interrupted'
                    OR (status = 'failed' AND next_retry_at IS NOT NULL AND next_retry_at <= ?))
               AND retryable = 1
               AND auto_resume = 1
               AND dead_letter = 0
               AND parent_task_id IS NULL
             ORDER BY created_at ASC",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
    }

    /// Remove finished tasks in bounded batches while preserving parent/retry
    /// records that are still referenced by another task.
    pub async fn purge_finished_before(&self, before: &str, limit: i64) -> Result<u64, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT task.id
             FROM background_tasks AS task
             WHERE task.status IN ('succeeded', 'failed', 'cancelled')
               AND task.completed_at IS NOT NULL
               AND task.completed_at < ?
               AND NOT EXISTS (
                    SELECT 1 FROM background_tasks AS ref
                    WHERE ref.parent_task_id = task.id OR ref.retry_of = task.id
               )
             ORDER BY task.completed_at ASC
             LIMIT ?",
        )
        .bind(before)
        .bind(limit.max(1))
        .fetch_all(&mut *transaction)
        .await?;
        let mut deleted = 0;
        for id in ids {
            let result = sqlx::query("DELETE FROM background_tasks WHERE id = ?")
                .bind(id)
                .execute(&mut *transaction)
                .await?;
            deleted += result.rows_affected();
        }
        transaction.commit().await?;
        Ok(deleted)
    }
}

fn require_task_row(rows_affected: u64) -> Result<(), sqlx::Error> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(sqlx::Error::RowNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::TaskRepository;
    use crate::services::tasks::models::{TaskListFilter, TaskSpec, TASK_CANCELLED};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn repository() -> TaskRepository {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create task repository database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply task migrations");
        sqlx::query(
            "INSERT INTO kb_knowledge_bases (id, name, created_at, updated_at)
             VALUES ('kb-1', 'Task test knowledge base', 'now', 'now')",
        )
        .execute(&pool)
        .await
        .expect("create task test knowledge base");
        TaskRepository::new(pool)
    }

    fn document_task() -> TaskSpec {
        TaskSpec::new(
            "knowledge",
            "process_document",
            "knowledge_base",
            "kb-1",
        )
        .subject_id(Some("doc-1".to_string()))
        .idempotency_key("knowledge:document:doc-1")
        .payload(serde_json::json!({ "doc_id": "doc-1" }))
        .total_items(4)
    }

    #[tokio::test]
    async fn active_idempotency_key_allows_only_one_task() {
        let repository = repository().await;
        let first = repository
            .create_if_idle(&document_task())
            .await
            .expect("create first task")
            .expect("first task created");
        assert!(repository
            .create_if_idle(&document_task())
            .await
            .expect("create duplicate task")
            .is_none());
        assert!(repository.claim(&first.id, "processing").await.unwrap());
        repository.succeed(&first.id, Some("{\"chunks\":4}")).await.unwrap();
        assert!(repository
            .create_if_idle(&document_task())
            .await
            .expect("create replacement task")
            .is_some());
    }

    #[tokio::test]
    async fn running_task_can_be_cancelled_and_retried() {
        let repository = repository().await;
        let parent = repository
            .create_if_idle(
                &TaskSpec::new("knowledge", "import_source", "knowledge_base", "kb-1")
                    .subject_id(Some("source-1".to_string()))
                    .idempotency_key("knowledge:source:source-1"),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(repository.claim(&parent.id, "processing").await.unwrap());
        let task = repository
            .create_if_idle(
                &document_task().parent_task_id(Some(parent.id.clone())),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(repository.claim(&task.id, "embedding").await.unwrap());
        repository
            .update_progress(&task.id, "embedding", 50, 2, 4)
            .await
            .unwrap();
        let cancelling = repository.request_cancel(&task.id).await.unwrap();
        assert_eq!(cancelling.status, "running");
        assert_eq!(cancelling.cancel_requested, 1);
        assert_eq!(
            repository.ensure_not_cancelled(&task.id).await.unwrap_err(),
            TASK_CANCELLED
        );
        let cancelled = repository.get(&task.id).await.unwrap();
        assert_eq!(cancelled.status, "cancelled");

        let retry = repository.retry(&task.id).await.unwrap().unwrap();
        assert_eq!(retry.status, "pending");
        assert_eq!(retry.retry_of.as_deref(), Some(task.id.as_str()));
        assert_eq!(retry.subject_id.as_deref(), Some("doc-1"));
        assert_eq!(retry.parent_task_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(retry.attempt, 2);
    }

    #[tokio::test]
    async fn cancellation_wins_over_a_late_successful_completion() {
        let repository = repository().await;
        let task = repository
            .create_if_idle(&document_task())
            .await
            .unwrap()
            .unwrap();
        assert!(repository.claim(&task.id, "processing").await.unwrap());
        let cancelling = repository.request_cancel(&task.id).await.unwrap();
        assert_eq!(cancelling.status, "running");
        assert_eq!(cancelling.cancel_requested, 1);

        repository
            .succeed(&task.id, Some(r#"{"chunks":4}"#))
            .await
            .unwrap();

        let cancelled = repository.get(&task.id).await.unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.stage, "cancelled");
        assert_eq!(cancelled.cancel_requested, 1);
        assert!(cancelled.result_json.is_none());
        assert!(cancelled.next_retry_at.is_none());
    }

    #[tokio::test]
    async fn startup_marks_pending_and_running_tasks_interrupted() {
        let repository = repository().await;
        let pending = repository
            .create_if_idle(&document_task())
            .await
            .unwrap()
            .unwrap();
        let other = TaskSpec::new("wiki", "ingest", "wiki_project", "wiki-1")
            .subject_id(Some("source-1".to_string()))
            .idempotency_key("wiki:ingest:wiki-1:source-1");
        let running = repository.create_if_idle(&other).await.unwrap().unwrap();
        repository.claim(&running.id, "parsing").await.unwrap();

        assert_eq!(repository.interrupt_inflight().await.unwrap(), 2);
        assert_eq!(repository.get(&pending.id).await.unwrap().status, "interrupted");
        assert_eq!(repository.get(&running.id).await.unwrap().status, "interrupted");
        let interrupted = repository
            .list(&TaskListFilter {
                status: Some("interrupted".to_string()),
                ..TaskListFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(interrupted.len(), 2);
    }

    #[tokio::test]
    async fn lease_heartbeat_and_expiry_are_persisted() {
        let repository = repository().await;
        let task = repository.create_if_idle(&document_task()).await.unwrap().unwrap();
        assert!(repository.claim(&task.id, "processing").await.unwrap());
        let claimed = repository.get(&task.id).await.unwrap();
        assert!(claimed.lease_owner.is_some());
        assert!(claimed.lease_until.is_some());
        assert!(claimed.heartbeat_at.is_some());
        repository.heartbeat(&task.id).await.unwrap();

        sqlx::query(
            "UPDATE background_tasks
             SET lease_until = '2000-01-01T00:00:00Z'
             WHERE id = ?",
        )
        .bind(&task.id)
        .execute(&repository.pool)
        .await
        .unwrap();
        assert_eq!(repository.reap_expired_leases().await.unwrap(), 1);
        let expired = repository.get(&task.id).await.unwrap();
        assert_eq!(expired.status, "interrupted");
        assert!(expired.lease_owner.is_none());
    }

    #[tokio::test]
    async fn failed_tasks_back_off_and_then_move_to_dead_letter() {
        let repository = repository().await;
        let task = repository
            .create_if_idle(&document_task().auto_resume(true).max_attempts(2))
            .await
            .unwrap()
            .unwrap();
        assert!(repository.claim(&task.id, "processing").await.unwrap());
        repository.fail(&task.id, "temporary failure").await.unwrap();
        let first_failure = repository.get(&task.id).await.unwrap();
        assert_eq!(first_failure.dead_letter, 0);
        assert!(first_failure.next_retry_at.is_some());

        sqlx::query("UPDATE background_tasks SET next_retry_at = '2000-01-01T00:00:00Z' WHERE id = ?")
            .bind(&task.id)
            .execute(&repository.pool)
            .await
            .unwrap();
        assert_eq!(repository.list_auto_resumable_interrupted().await.unwrap().len(), 1);
        let retry = repository.retry(&task.id).await.unwrap().unwrap();
        assert_eq!(retry.attempt, 2);
        assert!(repository.claim(&retry.id, "processing").await.unwrap());
        repository.fail(&retry.id, "permanent failure").await.unwrap();
        let dead = repository.get(&retry.id).await.unwrap();
        assert_eq!(dead.dead_letter, 1);
        assert!(dead.next_retry_at.is_none());
        assert!(repository.retry(&retry.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn automatic_recovery_selects_only_retryable_root_tasks() {
        let repository = repository().await;
        let resumable = repository
            .create_if_idle(
                &document_task()
                    .idempotency_key("knowledge:document:resumable")
                    .auto_resume(true),
            )
            .await
            .unwrap()
            .unwrap();
        let manual = repository
            .create_if_idle(
                &document_task().idempotency_key("knowledge:document:manual"),
            )
            .await
            .unwrap()
            .unwrap();
        let non_retryable = repository
            .create_if_idle(
                &document_task()
                    .idempotency_key("knowledge:document:non-retryable")
                    .retryable(false)
                    .auto_resume(true),
            )
            .await
            .unwrap()
            .unwrap();
        let parent = repository
            .create_if_idle(
                &TaskSpec::new("knowledge", "import_source", "knowledge_base", "kb-1")
                    .subject_id(Some("source-1".to_string()))
                    .idempotency_key("knowledge:source:parent"),
            )
            .await
            .unwrap()
            .unwrap();
        let child = repository
            .create_if_idle(
                &document_task()
                    .idempotency_key("knowledge:document:child")
                    .parent_task_id(Some(parent.id.clone()))
                    .auto_resume(true),
            )
            .await
            .unwrap()
            .unwrap();

        repository.interrupt_inflight().await.unwrap();
        let selected = repository
            .list_auto_resumable_interrupted()
            .await
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, resumable.id);
        assert_eq!(repository.get(&manual.id).await.unwrap().status, "interrupted");
        assert_eq!(
            repository.get(&non_retryable.id).await.unwrap().status,
            "interrupted"
        );
        assert_eq!(repository.get(&child.id).await.unwrap().status, "interrupted");
    }

    #[tokio::test]
    async fn retention_preserves_referenced_parent_tasks_until_children_are_removed() {
        let repository = repository().await;
        let standalone = repository
            .create_if_idle(
                &document_task().idempotency_key("knowledge:retention:standalone"),
            )
            .await
            .unwrap()
            .unwrap();
        repository.claim(&standalone.id, "processing").await.unwrap();
        repository.succeed(&standalone.id, None).await.unwrap();

        let parent = repository
            .create_if_idle(
                &TaskSpec::new("knowledge", "import_source", "knowledge_base", "kb-1")
                    .idempotency_key("knowledge:retention:parent"),
            )
            .await
            .unwrap()
            .unwrap();
        repository.claim(&parent.id, "processing").await.unwrap();
        repository.succeed(&parent.id, None).await.unwrap();
        let child = repository
            .create_if_idle(
                &document_task()
                    .idempotency_key("knowledge:retention:child")
                    .parent_task_id(Some(parent.id.clone())),
            )
            .await
            .unwrap()
            .unwrap();
        repository.claim(&child.id, "processing").await.unwrap();
        repository.succeed(&child.id, None).await.unwrap();
        sqlx::query(
            "UPDATE background_tasks SET completed_at = '2000-01-01T00:00:00Z', updated_at = completed_at",
        )
        .execute(&repository.pool)
        .await
        .unwrap();

        assert_eq!(repository.purge_finished_before("2020-01-01T00:00:00Z", 100).await.unwrap(), 2);
        assert!(repository.get(&standalone.id).await.is_err());
        assert!(repository.get(&child.id).await.is_err());
        assert!(repository.get(&parent.id).await.is_ok());
        assert_eq!(repository.purge_finished_before("2020-01-01T00:00:00Z", 100).await.unwrap(), 1);
        assert!(repository.get(&parent.id).await.is_err());
    }

    #[tokio::test]
    async fn retry_preserves_automatic_recovery_policy() {
        let repository = repository().await;
        let task = repository
            .create_if_idle(
                &document_task()
                    .idempotency_key("knowledge:document:retry-policy")
                    .auto_resume(true),
            )
            .await
            .unwrap()
            .unwrap();
        repository.fail(&task.id, "injected failure").await.unwrap();

        let retry = repository.retry(&task.id).await.unwrap().unwrap();
        assert_eq!(retry.retry_of.as_deref(), Some(task.id.as_str()));
        assert_eq!(retry.auto_resume, 1);
        assert_eq!(retry.attempt, 2);
    }
}
