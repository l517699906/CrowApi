pub mod models;
pub mod repository;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
    SqliteSynchronous,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const SECURE_SECRET_MIGRATION_VERSIONS: [i64; 2] = [20, 22];

fn sqlite_connect_options(db_url: &str) -> SqliteConnectOptions {
    SqliteConnectOptions::from_str(db_url)
        .expect("failed to parse database URL")
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
}

pub struct Database {
    pub pool: SqlitePool,
}

async fn secure_secret_migration_pending(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let migration_table_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await?;
    if migration_table_exists == 0 {
        return Ok(true);
    }
    let applied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE version IN (?, ?) AND success = 1",
    )
    .bind(SECURE_SECRET_MIGRATION_VERSIONS[0])
    .bind(SECURE_SECRET_MIGRATION_VERSIONS[1])
    .fetch_one(pool)
    .await?;
    Ok(applied < SECURE_SECRET_MIGRATION_VERSIONS.len() as i64)
}

async fn backup_before_secure_secret_migration(
    pool: &SqlitePool,
    app_data_dir: &Path,
) -> Result<PathBuf, String> {
    let backup_dir = app_data_dir.join("backups");
    std::fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("failed to create database backup directory: {}", error))?;
    let filename = format!(
        "crowapi-before-secret-migration-{}.db",
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f")
    );
    let path = backup_dir.join(filename);
    let escaped = path.to_string_lossy().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{}'", escaped))
        .execute(pool)
        .await
        .map_err(|error| format!("failed to create database migration backup: {}", error))?;
    Ok(path)
}

impl Database {
    pub async fn new(app: &AppHandle) -> Self {
        // 获取应用数据目录（macOS: ~/Library/Application Support/com.llf.crowapi）
        let app_data_dir = app
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");

        std::fs::create_dir_all(&app_data_dir).expect("failed to create app data dir");

        if let Some(rollback_path) = crate::commands::backup::apply_pending_restore(&app_data_dir)
            .expect("failed to apply pending full restore")
        {
            log::warn!(
                "完整备份恢复成功，原数据保存在: {}",
                rollback_path.display()
            );
        }

        // mode=rwc：不存在则创建
        let db_path = app_data_dir.join("crowapi.db");
        let existing_database = db_path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

        let connect_options = sqlite_connect_options(&db_url);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(connect_options)
            .await
            .expect("failed to connect to database");

        if existing_database
            && secure_secret_migration_pending(&pool)
                .await
                .expect("failed to inspect secure secret migration state")
        {
            let backup_path = backup_before_secure_secret_migration(&pool, &app_data_dir)
                .await
                .expect("failed to back up database before secure secret migration");
            log::warn!(
                "已在密钥迁移前备份数据库: {}",
                backup_path.display()
            );
        }

        // 执行 migrations（编译时嵌入 SQL 文件）
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("failed to run database migrations");

        // Migration 026 creates the Wiki FTS projection, but page bodies live
        // in managed files and therefore cannot be backfilled by SQL alone.
        // Reconcile missing/orphaned rows before the first search request.
        if let Err(error) = crate::services::wiki::repository::rebuild_search_index(&pool).await {
            log::warn!("Wiki 搜索索引重建失败，将在后续页面写入时自动补齐: {}", error);
        }

        let active_key_version: i64 = sqlx::query_scalar(
            "SELECT active_key_version FROM secret_store_metadata WHERE singleton = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("failed to read active master key version");
        let secret_store = crate::core::secret_store::default_secret_store();
        let used_key_versions: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT key_version FROM secure_secrets ORDER BY key_version",
        )
        .fetch_all(&pool)
        .await
        .expect("failed to read encrypted secret key versions");
        for key_version in used_key_versions {
            secret_store
                .ensure_key_version(key_version)
                .unwrap_or_else(|error| {
                    panic!(
                        "master key version {} required by encrypted data is unavailable: {}",
                        key_version, error
                    )
                });
        }
        secret_store
            .set_active_key_version(active_key_version)
            .expect("failed to configure active master key version");

        let task_repository = crate::services::tasks::repository::TaskRepository::new(pool.clone());
        let expired = task_repository
            .reap_expired_leases()
            .await
            .expect("failed to reap expired background task leases");
        let interrupted = task_repository
            .interrupt_inflight()
            .await
            .expect("failed to recover interrupted background tasks");
        if expired > 0 || interrupted > 0 {
            log::warn!(
                "已将 {} 个租约过期、{} 个未完成后台任务标记为 interrupted",
                expired,
                interrupted
            );
        }

        let secret_report = repository::Repository::new(pool.clone())
            .migrate_legacy_secrets()
            .await
            .expect("failed to migrate legacy API keys");
        if secret_report.channels_migrated > 0
            || secret_report.api_keys_migrated > 0
        {
            log::warn!(
                "密钥迁移完成: 渠道 {}, 访问密钥 {}",
                secret_report.channels_migrated,
                secret_report.api_keys_migrated,
            );
        }

        // Seed built-in security rules if table exists and is empty
        let _ = crate::security::rules::seed_builtin_rules(&pool).await;

        Self { pool }
    }
}

#[cfg(test)]
mod tests {
    use super::sqlite_connect_options;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::migrate::MigrateError;

    #[tokio::test]
    async fn all_migrations_apply_cleanly_and_are_idempotent() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create migration test database");

        let migrator = sqlx::migrate!("./migrations");
        migrator.run(&pool).await.expect("apply all migrations");
        migrator.run(&pool).await.expect("reapply all migrations");

        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await
            .expect("check migrated database integrity");
        assert_eq!(integrity, "ok");

        sqlx::query(
            "INSERT INTO api_keys
             (id, name, key, status, allowed_models, allowed_channels, quota_limit, quota_used, created_at, updated_at)
             VALUES ('legacy-scope-key', 'legacy', 'legacy-secret', 1, '[]', '[]', 0, 0, 'now', 'now')",
        )
        .execute(&pool)
        .await
        .expect("insert API key through the pre-scope column contract");
        let access_scopes: String = sqlx::query_scalar(
            "SELECT access_scopes FROM api_keys WHERE id = 'legacy-scope-key'",
        )
        .fetch_one(&pool)
        .await
        .expect("read migrated access scope default");
        assert_eq!(access_scopes, "[\"gateway\"]");

        let audit_table: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'auth_audit_events'",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect authentication audit table");
        assert_eq!(audit_table, 1);
    }

    #[tokio::test]
    async fn migration_checksum_drift_is_rejected() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create migration drift test database");
        let migrator = sqlx::migrate!("./migrations");
        migrator.run(&pool).await.expect("apply migrations");
        sqlx::query("UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 20")
            .execute(&pool)
            .await
            .expect("tamper migration checksum");

        let error = migrator
            .run(&pool)
            .await
            .expect_err("checksum drift must block startup migration");

        assert!(matches!(error, MigrateError::VersionMismatch(20)));
    }

    #[tokio::test]
    async fn sqlite_connections_enable_foreign_keys_and_busy_timeout() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlite_connect_options("sqlite::memory:"))
            .await
            .expect("create configured SQLite pool");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("read foreign key pragma");
        let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .expect("read busy timeout pragma");
        assert_eq!(foreign_keys, 1);
        assert_eq!(busy_timeout, 5_000);
    }
}
