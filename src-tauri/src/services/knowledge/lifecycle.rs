use super::{models::KbKnowledgeBase, retriever, storage};
use crate::db::models::now_iso;
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use thiserror::Error;

const CLEANUP_TASK_DOMAIN: &str = "maintenance";
const CLEANUP_TASK_TYPE: &str = "cleanup_staged_path";

#[derive(Debug, Error)]
pub enum DeleteKnowledgeBaseError {
    #[error("knowledge base not found")]
    NotFound,
    #[error("invalid knowledge base resource: {0}")]
    InvalidResource(String),
    #[error("knowledge base database operation failed: {0}")]
    Database(#[source] sqlx::Error),
    #[error("failed to stage knowledge base files: {0}")]
    Stage(String),
    #[error("delete failed: {cause}; rollback was incomplete: {rollback}")]
    Rollback { cause: String, rollback: String },
}

#[derive(Debug)]
pub struct DeleteKnowledgeBaseOutcome {
    pub knowledge_base: KbKnowledgeBase,
}

#[derive(Debug)]
struct StagedRemoval {
    original: PathBuf,
    staged: PathBuf,
}

pub async fn delete_knowledge_base(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
) -> Result<DeleteKnowledgeBaseOutcome, DeleteKnowledgeBaseError> {
    let mut managed_paths = vec![
        storage::kb_storage_dir(app, kb_id)
            .map_err(DeleteKnowledgeBaseError::InvalidResource)?,
    ];
    managed_paths.extend(
        retriever::index_artifact_paths(kb_id)
            .map_err(DeleteKnowledgeBaseError::InvalidResource)?,
    );

    let (outcome, staged) = commit_knowledge_base_delete(pool, kb_id, managed_paths).await?;
    for staged_path in finalize_staged(staged, kb_id).await {
        schedule_staged_cleanup(pool, app, kb_id, staged_path).await;
    }

    // A worker that was already between cancellation checkpoints may have
    // recreated a managed path. A final bounded sweep keeps those late files
    // from surviving a successful database deletion.
    if let Err(error) = storage::remove_kb_storage(app, kb_id).await {
        tracing::warn!(%error, knowledge_base_id = %kb_id, "failed to sweep knowledge base storage after deletion");
    }
    match retriever::index_artifact_paths(kb_id) {
        Ok(paths) => finalize_paths(paths, kb_id, "late index artifact").await,
        Err(error) => tracing::warn!(%error, knowledge_base_id = %kb_id, "failed to inspect late knowledge index artifacts"),
    }

    Ok(outcome)
}

#[cfg(test)]
async fn delete_knowledge_base_with_paths(
    pool: &SqlitePool,
    kb_id: &str,
    managed_paths: Vec<PathBuf>,
) -> Result<DeleteKnowledgeBaseOutcome, DeleteKnowledgeBaseError> {
    let (outcome, staged) = commit_knowledge_base_delete(pool, kb_id, managed_paths).await?;
    let _ = finalize_staged(staged, kb_id).await;
    Ok(outcome)
}

async fn commit_knowledge_base_delete(
    pool: &SqlitePool,
    kb_id: &str,
    managed_paths: Vec<PathBuf>,
) -> Result<(DeleteKnowledgeBaseOutcome, Vec<StagedRemoval>), DeleteKnowledgeBaseError> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(DeleteKnowledgeBaseError::Database)?;

    let knowledge_base = sqlx::query_as::<_, KbKnowledgeBase>(
        "SELECT * FROM kb_knowledge_bases WHERE id = ?",
    )
    .bind(kb_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(DeleteKnowledgeBaseError::Database)?
    .ok_or(DeleteKnowledgeBaseError::NotFound)?;

    // Acquire SQLite's writer lock before touching files. New task inserts then
    // wait until this transaction either rolls back or commits the deletion.
    let locked = sqlx::query(
        "UPDATE kb_knowledge_bases SET updated_at = updated_at WHERE id = ?",
    )
    .bind(kb_id)
    .execute(&mut *transaction)
    .await
    .map_err(DeleteKnowledgeBaseError::Database)?;
    if locked.rows_affected() != 1 {
        return Err(DeleteKnowledgeBaseError::NotFound);
    }

    let staged = match stage_paths(managed_paths).await {
        Ok(staged) => staged,
        Err(error) => {
            transaction.rollback().await.ok();
            return Err(DeleteKnowledgeBaseError::Stage(error));
        }
    };

    if let Err(error) = cancel_task_tree(&mut transaction, kb_id).await {
        return Err(rollback_and_restore(transaction, staged, error.to_string()).await);
    }

    let deleted = match sqlx::query("DELETE FROM kb_knowledge_bases WHERE id = ?")
        .bind(kb_id)
        .execute(&mut *transaction)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            return Err(rollback_and_restore(transaction, staged, error.to_string()).await);
        }
    };
    if deleted.rows_affected() != 1 {
        return Err(
            rollback_and_restore(transaction, staged, "knowledge base disappeared".to_string())
                .await,
        );
    }

    if let Err(error) = transaction.commit().await {
        let restore = restore_staged(&staged).await;
        return Err(match restore {
            Ok(()) => DeleteKnowledgeBaseError::Database(error),
            Err(restore_error) => DeleteKnowledgeBaseError::Rollback {
                cause: error.to_string(),
                rollback: restore_error,
            },
        });
    }

    Ok((DeleteKnowledgeBaseOutcome { knowledge_base }, staged))
}

async fn cancel_task_tree(
    transaction: &mut Transaction<'_, Sqlite>,
    kb_id: &str,
) -> Result<(), sqlx::Error> {
    let now = now_iso();
    sqlx::query(
        "WITH RECURSIVE task_tree(id) AS (
             SELECT id
             FROM background_tasks
             WHERE domain = 'knowledge'
               AND resource_type = 'knowledge_base'
               AND resource_id = ?
             UNION
             SELECT child.id
             FROM background_tasks AS child
             JOIN task_tree AS parent ON child.parent_task_id = parent.id
         )
         UPDATE background_tasks
         SET cancel_requested = 1,
             status = CASE WHEN status = 'pending' THEN 'cancelled' ELSE status END,
             stage = CASE WHEN status = 'pending' THEN 'cancelled' ELSE stage END,
             completed_at = CASE WHEN status = 'pending' THEN ? ELSE completed_at END,
             auto_resume = 0,
             next_retry_at = NULL,
             dead_letter = 0,
             lease_owner = NULL,
             lease_until = NULL,
             heartbeat_at = ?,
             updated_at = ?
         WHERE id IN (SELECT id FROM task_tree)
           AND status IN ('pending', 'running')",
    )
    .bind(kb_id)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn stage_paths(paths: Vec<PathBuf>) -> Result<Vec<StagedRemoval>, String> {
    let mut staged = Vec::new();
    for path in paths {
        match stage_path(path).await {
            Ok(Some(removal)) => staged.push(removal),
            Ok(None) => {}
            Err(error) => {
                let restore_error = restore_staged(&staged).await.err();
                return Err(match restore_error {
                    Some(restore_error) => format!("{}; restore failed: {}", error, restore_error),
                    None => error,
                });
            }
        }
    }
    Ok(staged)
}

async fn stage_path(path: PathBuf) -> Result<Option<StagedRemoval>, String> {
    match tokio::fs::metadata(&path).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect {}: {}", path.display(), error)),
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("managed path has no parent: {}", path.display()))?;
    let staged = parent.join(format!(".crowapi-delete-{}", uuid::Uuid::new_v4()));
    tokio::fs::rename(&path, &staged)
        .await
        .map_err(|error| format!("failed to stage {}: {}", path.display(), error))?;
    Ok(Some(StagedRemoval {
        original: path,
        staged,
    }))
}

async fn restore_staged(removals: &[StagedRemoval]) -> Result<(), String> {
    let mut errors = Vec::new();
    for removal in removals.iter().rev() {
        if let Err(error) = tokio::fs::rename(&removal.staged, &removal.original).await {
            errors.push(format!("{}: {}", removal.original.display(), error));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

async fn rollback_and_restore(
    transaction: Transaction<'_, Sqlite>,
    staged: Vec<StagedRemoval>,
    cause: String,
) -> DeleteKnowledgeBaseError {
    let rollback_error = transaction.rollback().await.err().map(|error| error.to_string());
    let restore_error = restore_staged(&staged).await.err();
    match (rollback_error, restore_error) {
        (None, None) => DeleteKnowledgeBaseError::Database(sqlx::Error::Protocol(cause)),
        (rollback_error, restore_error) => DeleteKnowledgeBaseError::Rollback {
            cause,
            rollback: [rollback_error, restore_error]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; "),
        },
    }
}

async fn finalize_staged(removals: Vec<StagedRemoval>, kb_id: &str) -> Vec<PathBuf> {
    let mut failures = Vec::new();
    for removal in removals {
        if let Err(error) = remove_path(&removal.staged).await {
            tracing::warn!(
                %error,
                knowledge_base_id = %kb_id,
                original_path = %removal.original.display(),
                staged_path = %removal.staged.display(),
                "knowledge base was deleted but staged file cleanup failed"
            );
            failures.push(removal.staged);
        }
    }
    failures
}

async fn finalize_paths(paths: Vec<PathBuf>, kb_id: &str, kind: &str) {
    for path in paths {
        if let Err(error) = remove_path(&path).await {
            tracing::warn!(%error, knowledge_base_id = %kb_id, path = %path.display(), %kind, "post-delete file cleanup failed");
        }
    }
}

async fn remove_path(path: &Path) -> Result<(), String> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect {}: {}", path.display(), error)),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        tokio::fs::remove_file(path)
            .await
            .map_err(|error| format!("failed to remove {}: {}", path.display(), error))
    } else if metadata.is_dir() {
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(|error| format!("failed to remove {}: {}", path.display(), error))
    } else {
        Err(format!("unsupported staged path type: {}", path.display()))
    }
}

async fn schedule_staged_cleanup(
    pool: &SqlitePool,
    app: &AppHandle,
    kb_id: &str,
    staged_path: PathBuf,
) {
    use crate::services::tasks::{models::TaskSpec, repository::TaskRepository};
    use sha2::{Digest, Sha256};

    let path = staged_path.to_string_lossy().to_string();
    let path_hash = format!("{:x}", Sha256::digest(path.as_bytes()));
    let spec = TaskSpec::new(
        CLEANUP_TASK_DOMAIN,
        CLEANUP_TASK_TYPE,
        "knowledge_base_cleanup",
        kb_id,
    )
    .idempotency_key(format!("maintenance:cleanup:{}", path_hash))
    .payload(serde_json::json!({
        "payload_version": 1,
        "operation": CLEANUP_TASK_TYPE,
        "staged_path": path,
    }))
    .auto_resume(true)
    .max_attempts(8);
    let task = match TaskRepository::new(pool.clone()).create_if_idle(&spec).await {
        Ok(Some(task)) => task,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(%error, knowledge_base_id = %kb_id, path = %staged_path.display(), "failed to record staged cleanup task");
            return;
        }
    };
    if let Err(error) = crate::services::tasks::dispatcher::dispatch_existing(pool, app, &task.id).await {
        tracing::warn!(%error, task_id = %task.id, knowledge_base_id = %kb_id, "staged cleanup task was queued for retry");
    }
}

fn cleanup_path_has_safe_shape(path: &Path, allowed_roots: &[PathBuf]) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(filename) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(id) = filename.strip_prefix(".crowapi-delete-") else {
        return false;
    };
    uuid::Uuid::parse_str(id).is_ok() && allowed_roots.iter().any(|root| parent == root)
}

fn parse_cleanup_task(
    task: &crate::services::tasks::models::BackgroundTask,
) -> Result<PathBuf, String> {
    if task.domain != CLEANUP_TASK_DOMAIN
        || task.task_type != CLEANUP_TASK_TYPE
        || task.resource_type != "knowledge_base_cleanup"
    {
        return Err("后台任务不是知识库暂存文件清理任务".to_string());
    }
    let payload: serde_json::Value = serde_json::from_str(&task.payload_json)
        .map_err(|error| format!("清理任务参数损坏: {}", error))?;
    if payload.get("payload_version").and_then(serde_json::Value::as_u64) != Some(1)
        || payload.get("operation").and_then(serde_json::Value::as_str)
            != Some(CLEANUP_TASK_TYPE)
    {
        return Err("清理任务参数版本或操作不受支持".to_string());
    }
    payload
        .get("staged_path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "清理任务缺少暂存路径".to_string())
}

pub async fn start_existing_cleanup_task(
    pool: &SqlitePool,
    app: &AppHandle,
    task_id: &str,
) -> Result<(), String> {
    use crate::services::tasks::{emit_task_event, repository::TaskRepository};

    let tasks = TaskRepository::new(pool.clone());
    let task = tasks.get(task_id).await.map_err(|error| error.to_string())?;
    if task.status != "pending" {
        return Err("知识库暂存文件清理任务已经开始或结束".to_string());
    }
    let staged_path = parse_cleanup_task(&task)?;
    let allowed_roots = [
        storage::kb_storage_root(app)?,
        retriever::index_storage_dir(),
    ];
    if !cleanup_path_has_safe_shape(&staged_path, &allowed_roots) {
        return Err("清理任务路径不属于 CrowAPI 管理的暂存目录".to_string());
    }
    if !tasks
        .claim(task_id, "cleanup")
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("知识库暂存文件清理任务已经开始或结束".to_string());
    }
    match remove_path(&staged_path).await {
        Ok(()) => {
            tasks
                .succeed(task_id, Some(r#"{"removed":true}"#))
                .await
                .map_err(|error| error.to_string())?;
            if let Ok(completed) = tasks.get(task_id).await {
                emit_task_event(app, &completed, Some("暂存文件清理完成"));
            }
            Ok(())
        }
        Err(error) => {
            let _ = tasks.fail(task_id, &error).await;
            if let Ok(failed) = tasks.get(task_id).await {
                emit_task_event(app, &failed, Some(&error));
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_path_has_safe_shape, delete_knowledge_base_with_paths,
        DeleteKnowledgeBaseError,
    };
    use crate::services::knowledge::{models::CreateKbInput, repository::KbRepository};
    use crate::services::tasks::{models::TaskSpec, repository::TaskRepository};
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
    use std::path::PathBuf;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create lifecycle test database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply lifecycle migrations");
        pool
    }

    async fn knowledge_base(pool: &SqlitePool) -> crate::services::knowledge::models::KbKnowledgeBase {
        KbRepository::new(pool.clone())
            .create_kb(&CreateKbInput {
                name: "Lifecycle".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .expect("create knowledge base")
    }

    #[tokio::test]
    async fn delete_cancels_the_full_task_tree_and_removes_staged_files() {
        let pool = pool().await;
        let kb = knowledge_base(&pool).await;
        let tasks = TaskRepository::new(pool.clone());
        let parent = tasks
            .create_if_idle(
                &TaskSpec::new("knowledge", "import_source", "knowledge_base", &kb.id)
                    .idempotency_key(format!("knowledge:source:{}", kb.id)),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(tasks.claim(&parent.id, "importing").await.unwrap());
        let child = tasks
            .create_if_idle(
                &TaskSpec::new("knowledge", "process_document", "knowledge_document", "doc-1")
                    .parent_task_id(Some(parent.id.clone()))
                    .idempotency_key("knowledge:document:doc-1"),
            )
            .await
            .unwrap()
            .unwrap();
        let direct = tasks
            .create_if_idle(
                &TaskSpec::new("knowledge", "build_index", "knowledge_base", &kb.id)
                    .idempotency_key(format!("knowledge:index:{}", kb.id)),
            )
            .await
            .unwrap()
            .unwrap();

        let root = std::env::temp_dir().join(format!("crowapi-lifecycle-{}", uuid::Uuid::new_v4()));
        let storage = root.join("kb-storage");
        let index = root.join("kb-index.hnsw");
        tokio::fs::create_dir_all(&storage).await.unwrap();
        tokio::fs::write(storage.join("document.snapshot"), b"managed").await.unwrap();
        tokio::fs::write(&index, b"index").await.unwrap();

        let outcome = delete_knowledge_base_with_paths(
            &pool,
            &kb.id,
            vec![storage.clone(), index.clone()],
        )
        .await
        .unwrap();
        assert_eq!(outcome.knowledge_base.id, kb.id);
        assert!(!storage.exists());
        assert!(!index.exists());
        assert!(matches!(
            KbRepository::new(pool.clone()).get_kb(&kb.id).await,
            Err(sqlx::Error::RowNotFound)
        ));

        let parent = tasks.get(&parent.id).await.unwrap();
        assert_eq!(parent.status, "running");
        assert_eq!(parent.cancel_requested, 1);
        assert_eq!(parent.auto_resume, 0);
        let child = tasks.get(&child.id).await.unwrap();
        assert_eq!(child.status, "cancelled");
        assert_eq!(child.cancel_requested, 1);
        let direct = tasks.get(&direct.id).await.unwrap();
        assert_eq!(direct.status, "cancelled");
        assert_eq!(direct.cancel_requested, 1);

        let late = tasks
            .create_if_idle(
                &TaskSpec::new("knowledge", "build_index", "knowledge_base", &kb.id)
                    .idempotency_key(format!("knowledge:late:{}", kb.id)),
            )
            .await;
        assert!(late.is_err());
        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn database_failure_restores_files_and_task_state() {
        let pool = pool().await;
        let kb = knowledge_base(&pool).await;
        let tasks = TaskRepository::new(pool.clone());
        let task = tasks
            .create_if_idle(
                &TaskSpec::new("knowledge", "build_index", "knowledge_base", &kb.id)
                    .idempotency_key(format!("knowledge:index:{}", kb.id)),
            )
            .await
            .unwrap()
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_kb_delete
             BEFORE DELETE ON kb_knowledge_bases
             BEGIN
                 SELECT RAISE(ABORT, 'injected delete failure');
             END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let root = std::env::temp_dir().join(format!("crowapi-lifecycle-{}", uuid::Uuid::new_v4()));
        let managed = root.join("managed");
        tokio::fs::create_dir_all(&managed).await.unwrap();
        tokio::fs::write(managed.join("document.snapshot"), b"managed").await.unwrap();

        assert!(delete_knowledge_base_with_paths(&pool, &kb.id, vec![managed.clone()])
            .await
            .is_err());
        assert!(managed.join("document.snapshot").exists());
        assert!(KbRepository::new(pool.clone()).get_kb(&kb.id).await.is_ok());
        let task = tasks.get(&task.id).await.unwrap();
        assert_eq!(task.status, "pending");
        assert_eq!(task.cancel_requested, 0);
        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn missing_knowledge_base_does_not_touch_files() {
        let pool = pool().await;
        let root = std::env::temp_dir().join(format!("crowapi-lifecycle-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let error = delete_knowledge_base_with_paths(&pool, "missing", vec![root.clone()])
            .await
            .unwrap_err();
        assert!(matches!(error, DeleteKnowledgeBaseError::NotFound));
        assert!(root.exists());
        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[test]
    fn cleanup_path_only_accepts_direct_staging_children_of_managed_roots() {
        let uuid = "e70d60f9-80b0-4ec2-a31d-31e544538ed7";
        let files_root = PathBuf::from("/crowapi/kb_files");
        let indexes_root = PathBuf::from("/crowapi/hnsw_indexes");
        let roots = [files_root.clone(), indexes_root.clone()];

        assert!(cleanup_path_has_safe_shape(
            &files_root.join(format!(".crowapi-delete-{uuid}")),
            &roots,
        ));
        assert!(cleanup_path_has_safe_shape(
            &indexes_root.join(format!(".crowapi-delete-{uuid}")),
            &roots,
        ));
        assert!(!cleanup_path_has_safe_shape(
            &files_root.join(format!("crowapi-delete-{uuid}")),
            &roots,
        ));
        assert!(!cleanup_path_has_safe_shape(
            &files_root.join(".crowapi-delete-not-a-uuid"),
            &roots,
        ));
        assert!(!cleanup_path_has_safe_shape(
            &files_root
                .join("nested")
                .join(format!(".crowapi-delete-{uuid}")),
            &roots,
        ));
        assert!(!cleanup_path_has_safe_shape(
            &PathBuf::from("/other/root").join(format!(".crowapi-delete-{uuid}")),
            &roots,
        ));
    }
}
