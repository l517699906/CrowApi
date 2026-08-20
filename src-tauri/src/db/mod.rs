pub mod models;
pub mod repository;

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

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
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 20 AND success = 1",
    )
    .fetch_one(pool)
    .await?;
    Ok(applied == 0)
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

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
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
        sqlx::query("UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 5")
            .execute(&pool)
            .await
            .expect("tamper migration checksum");

        let error = migrator
            .run(&pool)
            .await
            .expect_err("checksum drift must block startup migration");

        assert!(matches!(error, MigrateError::VersionMismatch(5)));
    }
}
