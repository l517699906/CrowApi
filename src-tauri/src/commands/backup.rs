use crate::core::error::{CommandError, CommandResult, CommandResultExt};
use crate::core::secret_store::{default_secret_store, key_preview_parts};
use crate::db::repository::Repository;
use crate::AppState;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::Manager;
use zeroize::Zeroizing;

const BACKUP_MAGIC: &str = "CROWAPI_FULL_BACKUP";
const BACKUP_VERSION: u32 = 1;
const BACKUP_AAD: &[u8] = b"CrowAPI full backup v1";
const KDF_MEMORY_KIB: u32 = 64 * 1024;
const KDF_ITERATIONS: u32 = 3;
const KDF_PARALLELISM: u32 = 1;
const MAX_BACKUP_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PAYLOAD_JSON_BYTES: u64 = MAX_BACKUP_BYTES * 2;
const MAX_BACKUP_FILES: usize = 100_000;
const MIN_RESTORE_SCHEMA_VERSION: i64 = 20;
const CURRENT_SCHEMA_VERSION: i64 = 20;

fn current_schema_version() -> i64 {
    CURRENT_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub created_at: String,
    pub app_version: String,
    #[serde(default = "current_schema_version")]
    pub schema_version: i64,
    pub database_bytes: u64,
    pub file_count: usize,
    pub file_bytes: u64,
    pub channel_count: i64,
    pub api_key_count: i64,
    pub knowledge_base_count: i64,
    pub wiki_project_count: i64,
    pub includes_logs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub selection_id: String,
    pub summary: BackupSummary,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupWriteResult {
    pub path: String,
    pub summary: BackupSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreScheduleResult {
    pub restart_required: bool,
    pub rollback_directory: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupEnvelope {
    magic: String,
    version: u32,
    created_at: String,
    cipher: String,
    kdf: KdfMetadata,
    salt: String,
    nonce: String,
    payload_sha256: String,
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct KdfMetadata {
    name: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupPayload {
    summary: BackupSummary,
    database: BackupBlob,
    files: Vec<BackupFile>,
    channel_secrets: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupBlob {
    sha256: String,
    data: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupFile {
    path: String,
    sha256: String,
    data: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingRestore {
    token: String,
    keep_local_settings: bool,
}

fn selected_backups() -> &'static Mutex<HashMap<String, PathBuf>> {
    static SELECTED: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    SELECTED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn validate_password(password: &str) -> CommandResult<()> {
    if password.chars().count() < 10 {
        return Err(CommandError::validation("备份口令至少需要 10 个字符"));
    }
    Ok(())
}

fn derive_backup_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let params = Params::new(
        KDF_MEMORY_KIB,
        KDF_ITERATIONS,
        KDF_PARALLELISM,
        Some(32),
    )
    .map_err(|error| error.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0_u8; 32]);
    argon2
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|error| error.to_string())?;
    Ok(key)
}

fn encrypt_payload(payload: &BackupPayload, password: &str) -> Result<Vec<u8>, String> {
    validate_payload(payload)?;
    let serialized = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let compressed = zstd::stream::encode_all(serialized.as_slice(), 6)
        .map_err(|error| error.to_string())?;
    let payload_sha256 = hex::encode(Sha256::digest(&serialized));
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 24];
    rand::rng().fill_bytes(&mut salt);
    rand::rng().fill_bytes(&mut nonce);
    let key = derive_backup_key(password, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| "无法初始化备份加密器".to_string())?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &compressed,
                aad: BACKUP_AAD,
            },
        )
        .map_err(|_| "加密备份失败".to_string())?;
    let envelope = BackupEnvelope {
        magic: BACKUP_MAGIC.to_string(),
        version: BACKUP_VERSION,
        created_at: payload.summary.created_at.clone(),
        cipher: "xchacha20poly1305".to_string(),
        kdf: KdfMetadata {
            name: "argon2id".to_string(),
            memory_kib: KDF_MEMORY_KIB,
            iterations: KDF_ITERATIONS,
            parallelism: KDF_PARALLELISM,
        },
        salt: base64::engine::general_purpose::STANDARD.encode(salt),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        payload_sha256,
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    };
    serde_json::to_vec_pretty(&envelope).map_err(|error| error.to_string())
}

fn decrypt_payload(content: &[u8], password: &str) -> Result<BackupPayload, String> {
    if content.len() as u64 > MAX_BACKUP_BYTES {
        return Err("备份文件超过 1 GB 限制".to_string());
    }
    let envelope: BackupEnvelope =
        serde_json::from_slice(content).map_err(|_| "备份文件格式无效".to_string())?;
    if envelope.magic != BACKUP_MAGIC
        || envelope.version != BACKUP_VERSION
        || envelope.cipher != "xchacha20poly1305"
        || envelope.kdf.name != "argon2id"
        || envelope.kdf.memory_kib != KDF_MEMORY_KIB
        || envelope.kdf.iterations != KDF_ITERATIONS
        || envelope.kdf.parallelism != KDF_PARALLELISM
    {
        return Err("不支持的备份格式或加密参数".to_string());
    }
    let salt = base64::engine::general_purpose::STANDARD
        .decode(envelope.salt)
        .map_err(|_| "备份盐值格式无效".to_string())?;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(envelope.nonce)
        .map_err(|_| "备份随机数格式无效".to_string())?;
    if salt.len() != 16 || nonce.len() != 24 {
        return Err("备份加密参数长度无效".to_string());
    }
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(envelope.ciphertext)
        .map_err(|_| "备份密文格式无效".to_string())?;
    let key = derive_backup_key(password, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| "无法初始化备份解密器".to_string())?;
    let compressed = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: BACKUP_AAD,
            },
        )
        .map_err(|_| "备份口令错误或文件已损坏".to_string())?;
    let decoder = zstd::stream::read::Decoder::new(compressed.as_slice())
        .map_err(|_| "备份压缩数据已损坏".to_string())?;
    let mut serialized = Vec::new();
    decoder
        .take(MAX_PAYLOAD_JSON_BYTES + 1)
        .read_to_end(&mut serialized)
        .map_err(|_| "备份压缩数据已损坏".to_string())?;
    if serialized.len() as u64 > MAX_PAYLOAD_JSON_BYTES {
        return Err("备份解压内容超过安全限制".to_string());
    }
    if hex::encode(Sha256::digest(&serialized)) != envelope.payload_sha256 {
        return Err("备份内容校验失败".to_string());
    }
    let payload: BackupPayload =
        serde_json::from_slice(&serialized).map_err(|_| "备份内容格式无效".to_string())?;
    validate_payload(&payload)?;
    Ok(payload)
}

fn validate_payload(payload: &BackupPayload) -> Result<(), String> {
    if payload.summary.schema_version < MIN_RESTORE_SCHEMA_VERSION
        || payload.summary.schema_version > CURRENT_SCHEMA_VERSION
    {
        return Err(format!(
            "不支持的数据库结构版本: {}",
            payload.summary.schema_version
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&payload.summary.created_at)
        .map_err(|_| "备份创建时间格式无效".to_string())?;
    if payload.files.len() > MAX_BACKUP_FILES {
        return Err("备份包含的文件数量超过限制".to_string());
    }
    let database = base64::engine::general_purpose::STANDARD
        .decode(&payload.database.data)
        .map_err(|_| "数据库快照格式无效".to_string())?;
    if hex::encode(Sha256::digest(&database)) != payload.database.sha256 {
        return Err("数据库快照校验失败".to_string());
    }
    if payload.summary.database_bytes != database.len() as u64
        || payload.summary.file_count != payload.files.len()
    {
        return Err("备份清单与实际内容不一致".to_string());
    }
    let mut total_size = database.len() as u64;
    let mut file_bytes = 0_u64;
    let mut paths = HashSet::with_capacity(payload.files.len());
    for file in &payload.files {
        validate_logical_path(&file.path)?;
        if !paths.insert(file.path.as_str()) {
            return Err(format!("备份包含重复路径: {}", file.path));
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(&file.data)
            .map_err(|_| format!("备份文件 {} 格式无效", file.path))?;
        if hex::encode(Sha256::digest(&data)) != file.sha256 {
            return Err(format!("备份文件 {} 校验失败", file.path));
        }
        file_bytes = file_bytes.saturating_add(data.len() as u64);
        total_size = total_size.saturating_add(data.len() as u64);
        if total_size > MAX_BACKUP_BYTES {
            return Err("备份解压后超过 1 GB 限制".to_string());
        }
    }
    if payload.summary.file_bytes != file_bytes {
        return Err("备份文件清单大小与实际内容不一致".to_string());
    }
    Ok(())
}

fn validate_logical_path(path: &str) -> Result<(), String> {
    let components = Path::new(path)
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_string_lossy().to_string()),
            _ => Err("备份包含不安全路径".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let allowed = match components.as_slice() {
        [root, settings] if root == "app" && settings == "settings.json" => true,
        [root, directory, tail @ ..]
            if root == "app" && directory == "kb_files" && !tail.is_empty() => true,
        [root, tail @ ..] if root == "wiki" && !tail.is_empty() => true,
        _ => false,
    };
    if !allowed {
        return Err("备份包含不安全路径".to_string());
    }
    Ok(())
}

async fn query_count(pool: &sqlx::SqlitePool, table: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", table))
        .fetch_one(pool)
        .await
}

async fn create_database_snapshot(
    pool: &sqlx::SqlitePool,
    path: &Path,
    include_logs: bool,
) -> Result<Vec<u8>, String> {
    let escaped = path.to_string_lossy().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{}'", escaped))
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    let url = format!("sqlite://{}?mode=rw", path.display());
    let snapshot_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .map_err(|error| error.to_string())?;
    if !include_logs {
        let mut transaction = snapshot_pool.begin().await.map_err(|error| error.to_string())?;
        sqlx::query("DELETE FROM request_security_findings")
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        sqlx::query("DELETE FROM request_logs")
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        transaction.commit().await.map_err(|error| error.to_string())?;
    }
    sqlx::query("UPDATE kb_knowledge_bases SET index_status = CASE WHEN chunk_count > 0 THEN 'stale' ELSE 'none' END")
        .execute(&snapshot_pool)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query("UPDATE kb_index_meta SET status = 'stale', index_path = NULL")
        .execute(&snapshot_pool)
        .await
        .map_err(|error| error.to_string())?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&snapshot_pool)
        .await
        .map_err(|error| error.to_string())?;
    snapshot_pool.close().await;
    if integrity != "ok" {
        return Err(format!("数据库快照完整性检查失败: {}", integrity));
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(path);
    Ok(bytes)
}

fn collect_directory(
    root: &Path,
    logical_root: &str,
    files: &mut Vec<BackupFile>,
    total_bytes: &mut u64,
) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let logical = format!("{}/{}", logical_root, name);
        if metadata.is_dir() {
            collect_directory(&path, &logical, files, total_bytes)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        if files.len() >= MAX_BACKUP_FILES {
            return Err("待备份文件数量超过限制".to_string());
        }
        *total_bytes = total_bytes.saturating_add(metadata.len());
        if *total_bytes > MAX_BACKUP_BYTES {
            return Err("待备份数据超过 1 GB 限制".to_string());
        }
        let data = std::fs::read(&path).map_err(|error| error.to_string())?;
        files.push(BackupFile {
            path: logical,
            sha256: hex::encode(Sha256::digest(&data)),
            data: base64::engine::general_purpose::STANDARD.encode(data),
        });
    }
    Ok(())
}

async fn build_payload(
    app: &tauri::AppHandle,
    state: &AppState,
    include_logs: bool,
) -> Result<BackupPayload, String> {
    let app_data = app.path().app_data_dir().map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&app_data).map_err(|error| error.to_string())?;
    let snapshot_path = app_data.join(format!(
        ".crowapi-backup-{}.db",
        uuid::Uuid::new_v4()
    ));
    let database = create_database_snapshot(&state.db.pool, &snapshot_path, include_logs).await?;
    let mut files = Vec::new();
    let mut file_bytes = 0_u64;
    collect_directory(
        &app_data.join("kb_files"),
        "app/kb_files",
        &mut files,
        &mut file_bytes,
    )?;
    let settings_path = app_data.join("settings.json");
    if settings_path.is_file() {
        let data = std::fs::read(&settings_path).map_err(|error| error.to_string())?;
        file_bytes = file_bytes.saturating_add(data.len() as u64);
        files.push(BackupFile {
            path: "app/settings.json".to_string(),
            sha256: hex::encode(Sha256::digest(&data)),
            data: base64::engine::general_purpose::STANDARD.encode(data),
        });
    }
    collect_directory(
        &crate::services::wiki::project::wiki_base_dir(),
        "wiki",
        &mut files,
        &mut file_bytes,
    )?;
    let repository = Repository::new(state.db.pool.clone());
    let channels = repository
        .get_all_channels()
        .await
        .map_err(|error| error.to_string())?;
    let channel_secrets = channels
        .iter()
        .filter(|channel| !channel.api_key.is_empty())
        .map(|channel| (channel.id.clone(), channel.api_key.clone()))
        .collect();
    let schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&state.db.pool)
    .await
    .map_err(|error| error.to_string())?;
    let summary = BackupSummary {
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: app.package_info().version.to_string(),
        schema_version,
        database_bytes: database.len() as u64,
        file_count: files.len(),
        file_bytes,
        channel_count: query_count(&state.db.pool, "channels")
            .await
            .map_err(|error| error.to_string())?,
        api_key_count: query_count(&state.db.pool, "api_keys")
            .await
            .map_err(|error| error.to_string())?,
        knowledge_base_count: query_count(&state.db.pool, "kb_knowledge_bases")
            .await
            .map_err(|error| error.to_string())?,
        wiki_project_count: query_count(&state.db.pool, "wiki_projects")
            .await
            .map_err(|error| error.to_string())?,
        includes_logs: include_logs,
    };
    Ok(BackupPayload {
        summary,
        database: BackupBlob {
            sha256: hex::encode(Sha256::digest(&database)),
            data: base64::engine::general_purpose::STANDARD.encode(database),
        },
        files,
        channel_secrets,
    })
}

async fn save_backup_dialog(
    app: &tauri::AppHandle,
    content: Vec<u8>,
) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!(
            "crowapi-backup-{}.crowbackup",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ))
        .add_filter("CrowAPI encrypted backup", &["crowbackup"])
        .save_file(move |selected| {
            let result = selected
                .and_then(|file| file.into_path().ok())
                .map(|path| {
                    std::fs::write(&path, &content)
                        .map(|_| path)
                        .map_err(|error| error.to_string())
                })
                .transpose();
            let _ = tx.send(result);
        });
    rx.await.map_err(|error| error.to_string())?
}

async fn pick_backup_dialog(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("CrowAPI encrypted backup", &["crowbackup"])
        .pick_file(move |selected| {
            let result = selected
                .map(|file| file.into_path().map_err(|error| error.to_string()))
                .transpose();
            let _ = tx.send(result);
        });
    rx.await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn create_full_backup(
    password: String,
    include_logs: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<Option<BackupWriteResult>> {
    validate_password(&password)?;
    let payload = build_payload(&app, &state, include_logs)
        .await
        .map_err(|error| CommandError::reported("BACKUP_BUILD_FAILED", "生成完整备份失败", false, error))?;
    let content = encrypt_payload(&payload, &password)
        .map_err(|error| CommandError::reported("BACKUP_ENCRYPT_FAILED", "加密完整备份失败", false, error))?;
    let path = save_backup_dialog(&app, content)
        .await
        .command_error("BACKUP_WRITE_FAILED", "保存完整备份失败", false)?;
    Ok(path.map(|path| BackupWriteResult {
        path: path.to_string_lossy().to_string(),
        summary: payload.summary,
    }))
}

#[tauri::command]
pub async fn inspect_full_backup(
    password: String,
    app: tauri::AppHandle,
) -> CommandResult<Option<BackupPreview>> {
    validate_password(&password)?;
    let Some(path) = pick_backup_dialog(&app)
        .await
        .command_error("BACKUP_READ_FAILED", "选择完整备份失败", false)?
    else {
        return Ok(None);
    };
    let content = std::fs::read(&path)
        .map_err(|error| CommandError::reported("BACKUP_READ_FAILED", "读取完整备份失败", false, error))?;
    let payload = decrypt_payload(&content, &password)
        .map_err(|error| CommandError::new("BACKUP_DECRYPT_FAILED", error, false))?;
    let selection_id = uuid::Uuid::new_v4().to_string();
    let mut selections = selected_backups()
        .lock()
        .map_err(|_| CommandError::new("BACKUP_SELECTION_FAILED", "备份选择状态不可用", true))?;
    selections.clear();
    selections.insert(selection_id.clone(), path);
    Ok(Some(BackupPreview {
        selection_id,
        summary: payload.summary,
        warnings: vec![
            "恢复会在应用重启时替换本地数据库、知识库文件和 Wiki 文件".to_string(),
            "HNSW 索引不会从备份恢复，知识库会回退到线性检索并可重新构建索引".to_string(),
        ],
    }))
}

fn write_payload_to_stage(
    payload: &BackupPayload,
    app_stage: &Path,
    wiki_stage: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(app_stage).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(app_stage.join("kb_files"))
        .map_err(|error| error.to_string())?;
    std::fs::create_dir_all(wiki_stage).map_err(|error| error.to_string())?;
    let database = base64::engine::general_purpose::STANDARD
        .decode(&payload.database.data)
        .map_err(|_| "数据库快照格式无效".to_string())?;
    std::fs::write(app_stage.join("crowapi.db"), database)
        .map_err(|error| error.to_string())?;
    for file in &payload.files {
        validate_logical_path(&file.path)?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(&file.data)
            .map_err(|_| format!("备份文件 {} 格式无效", file.path))?;
        let relative = Path::new(&file.path);
        let mut components = relative.components();
        let root = components.next().expect("validated logical root");
        let tail: PathBuf = components.collect();
        let destination = match root {
            Component::Normal(value) if value == "app" => app_stage.join(tail),
            Component::Normal(value) if value == "wiki" => wiki_stage.join(tail),
            _ => return Err("备份包含无效文件根目录".to_string()),
        };
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(destination, data).map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn prepare_staged_database(
    database_path: &Path,
    channel_secrets: &HashMap<String, String>,
) -> Result<(), String> {
    let url = format!("sqlite://{}?mode=rw", database_path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .map_err(|error| error.to_string())?;
    let secret_store = default_secret_store();
    let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM secure_secrets WHERE owner_type = 'channel'")
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    for (channel_id, plaintext) in channel_secrets {
        let context = format!("channel:{}", channel_id);
        let encrypted = secret_store.encrypt(&context, plaintext)?;
        let (_, last_four) = key_preview_parts(plaintext);
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO secure_secrets (owner_type, owner_id, version, nonce, ciphertext, last_four, created_at, updated_at)
             VALUES ('channel', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(channel_id)
        .bind(encrypted.version)
        .bind(encrypted.nonce)
        .bind(encrypted.ciphertext)
        .bind(&last_four)
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        let updated = sqlx::query(
            "UPDATE channels SET api_key = ?, secret_ref = ?, api_key_last4 = ? WHERE id = ?",
        )
        .bind(format!("secret:{}", channel_id))
        .bind(&context)
        .bind(last_four)
        .bind(channel_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        if updated.rows_affected() != 1 {
            return Err(format!("备份包含未知渠道密钥: {}", channel_id));
        }
    }
    transaction.commit().await.map_err(|error| error.to_string())?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .map_err(|error| error.to_string())?;
    pool.close().await;
    if integrity != "ok" {
        return Err(format!("待恢复数据库完整性检查失败: {}", integrity));
    }
    Ok(())
}

#[tauri::command]
pub async fn schedule_full_restore(
    selection_id: String,
    password: String,
    keep_local_settings: bool,
    app: tauri::AppHandle,
) -> CommandResult<RestoreScheduleResult> {
    validate_password(&password)?;
    let path = selected_backups()
        .lock()
        .map_err(|_| CommandError::new("BACKUP_SELECTION_FAILED", "备份选择状态不可用", true))?
        .remove(&selection_id)
        .ok_or_else(|| CommandError::validation("备份选择已过期，请重新选择"))?;
    let content = std::fs::read(path)
        .map_err(|error| CommandError::reported("BACKUP_READ_FAILED", "读取完整备份失败", false, error))?;
    let payload = decrypt_payload(&content, &password)
        .map_err(|error| CommandError::new("BACKUP_DECRYPT_FAILED", error, false))?;
    let app_data = app.path().app_data_dir().map_err(|error| {
        CommandError::reported("APP_DATA_UNAVAILABLE", "无法获取应用数据目录", false, error)
    })?;
    if app_data.join("restore-pending.json").exists() {
        return Err(CommandError::new(
            "RESTORE_ALREADY_PENDING",
            "已有待执行的恢复任务，请先重启应用完成恢复",
            false,
        ));
    }
    let token = uuid::Uuid::new_v4().to_string();
    let app_stage = app_data.join(format!(".restore-{}", token));
    let wiki_base = crate::services::wiki::project::wiki_base_dir();
    let wiki_parent = wiki_base
        .parent()
        .ok_or_else(|| CommandError::new("RESTORE_STAGE_FAILED", "Wiki 数据目录无效", false))?;
    let wiki_stage = wiki_parent.join(format!(".restore-{}", token));
    if let Err(error) = write_payload_to_stage(&payload, &app_stage, &wiki_stage) {
        let _ = std::fs::remove_dir_all(&app_stage);
        let _ = std::fs::remove_dir_all(&wiki_stage);
        return Err(CommandError::reported(
            "RESTORE_STAGE_FAILED",
            "暂存恢复数据失败",
            false,
            error,
        ));
    }
    if let Err(error) =
        prepare_staged_database(&app_stage.join("crowapi.db"), &payload.channel_secrets).await
    {
        let _ = std::fs::remove_dir_all(&app_stage);
        let _ = std::fs::remove_dir_all(&wiki_stage);
        return Err(CommandError::reported(
            "RESTORE_STAGE_FAILED",
            "准备恢复数据库失败",
            false,
            error,
        ));
    }
    let pending = PendingRestore {
        token: token.clone(),
        keep_local_settings,
    };
    let marker = serde_json::to_vec_pretty(&pending)
        .command_error("RESTORE_STAGE_FAILED", "生成恢复标记失败", false)?;
    if let Err(error) = std::fs::write(app_data.join("restore-pending.json"), marker) {
        let _ = std::fs::remove_dir_all(&app_stage);
        let _ = std::fs::remove_dir_all(&wiki_stage);
        return Err(CommandError::reported(
            "RESTORE_STAGE_FAILED",
            "保存恢复标记失败",
            false,
            error,
        ));
    }
    Ok(RestoreScheduleResult {
        restart_required: true,
        rollback_directory: app_data
            .join("backups")
            .join(format!("restore-rollback-{}", token))
            .to_string_lossy()
            .to_string(),
    })
}

#[derive(Debug)]
struct RestoreOperation {
    name: &'static str,
    staged: PathBuf,
    target: PathBuf,
    rollback: PathBuf,
}

#[derive(Debug)]
struct AppliedRestoreOperation {
    staged: PathBuf,
    target: PathBuf,
    rollback: PathBuf,
    target_backed_up: bool,
    staged_installed: bool,
}

fn apply_restore_operation(operation: RestoreOperation) -> Result<AppliedRestoreOperation, String> {
    if operation.rollback.exists() {
        return Err(format!(
            "恢复回滚位置已存在: {}",
            operation.rollback.display()
        ));
    }
    let mut applied = AppliedRestoreOperation {
        staged: operation.staged,
        target: operation.target,
        rollback: operation.rollback,
        target_backed_up: false,
        staged_installed: false,
    };

    if applied.target.exists() {
        if let Some(parent) = applied.rollback.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::rename(&applied.target, &applied.rollback)
            .map_err(|error| error.to_string())?;
        applied.target_backed_up = true;
    }
    if applied.staged.exists() {
        if let Some(parent) = applied.target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if let Err(error) = std::fs::rename(&applied.staged, &applied.target) {
            if applied.target_backed_up {
                let _ = std::fs::rename(&applied.rollback, &applied.target);
            }
            return Err(error.to_string());
        }
        applied.staged_installed = true;
    }
    Ok(applied)
}

fn rollback_restore_operations(operations: &mut Vec<AppliedRestoreOperation>) -> Result<(), String> {
    let mut failures = Vec::new();
    while let Some(operation) = operations.pop() {
        if operation.staged_installed && operation.target.exists() {
            if let Some(parent) = operation.staged.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    failures.push(format!("无法重建恢复暂存目录: {}", error));
                    continue;
                }
            }
            if let Err(error) = std::fs::rename(&operation.target, &operation.staged) {
                failures.push(format!(
                    "无法撤回已安装数据 {}: {}",
                    operation.target.display(),
                    error
                ));
                continue;
            }
        }
        if operation.target_backed_up && operation.rollback.exists() {
            if let Some(parent) = operation.target.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    failures.push(format!("无法重建原数据目录: {}", error));
                    continue;
                }
            }
            if let Err(error) = std::fs::rename(&operation.rollback, &operation.target) {
                failures.push(format!(
                    "无法恢复原数据 {}: {}",
                    operation.target.display(),
                    error
                ));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn apply_pending_restore_with_wiki_base<F>(
    app_data: &Path,
    wiki_base: &Path,
    mut before_step: F,
) -> Result<Option<PathBuf>, String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let marker_path = app_data.join("restore-pending.json");
    if !marker_path.is_file() {
        return Ok(None);
    }
    let pending: PendingRestore = serde_json::from_slice(
        &std::fs::read(&marker_path).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if uuid::Uuid::parse_str(&pending.token).is_err() {
        return Err("恢复标记包含无效标识".to_string());
    }
    let app_stage = app_data.join(format!(".restore-{}", pending.token));
    let wiki_parent = wiki_base
        .parent()
        .ok_or_else(|| "Wiki 数据目录无效".to_string())?;
    let wiki_stage = wiki_parent.join(format!(".restore-{}", pending.token));
    if !app_stage.join("crowapi.db").is_file() {
        return Err("待恢复数据库不存在".to_string());
    }
    let rollback = app_data
        .join("backups")
        .join(format!("restore-rollback-{}", pending.token));
    std::fs::create_dir_all(&rollback).map_err(|error| error.to_string())?;

    let mut planned = vec![
        RestoreOperation {
            name: "database",
            staged: app_stage.join("crowapi.db"),
            target: app_data.join("crowapi.db"),
            rollback: rollback.join("crowapi.db"),
        },
        RestoreOperation {
            name: "database-wal",
            staged: app_stage.join("crowapi.db-wal"),
            target: app_data.join("crowapi.db-wal"),
            rollback: rollback.join("crowapi.db-wal"),
        },
        RestoreOperation {
            name: "database-shm",
            staged: app_stage.join("crowapi.db-shm"),
            target: app_data.join("crowapi.db-shm"),
            rollback: rollback.join("crowapi.db-shm"),
        },
        RestoreOperation {
            name: "knowledge-files",
            staged: app_stage.join("kb_files"),
            target: app_data.join("kb_files"),
            rollback: rollback.join("kb_files"),
        },
        RestoreOperation {
            name: "wiki-files",
            staged: wiki_stage.clone(),
            target: wiki_base.to_path_buf(),
            rollback: rollback.join("wiki"),
        },
    ];
    if !pending.keep_local_settings {
        planned.push(RestoreOperation {
            name: "settings",
            staged: app_stage.join("settings.json"),
            target: app_data.join("settings.json"),
            rollback: rollback.join("settings.json"),
        });
    }

    let mut applied = Vec::new();
    for operation in planned {
        let result = before_step(operation.name)
            .and_then(|_| apply_restore_operation(operation));
        match result {
            Ok(operation) => applied.push(operation),
            Err(error) => {
                let rollback_error = rollback_restore_operations(&mut applied).err();
                if rollback_error.is_none() {
                    let _ = std::fs::remove_dir_all(&rollback);
                }
                return Err(match rollback_error {
                    Some(rollback_error) => format!(
                        "应用恢复失败: {}; 回滚原数据失败: {}",
                        error, rollback_error
                    ),
                    None => format!("应用恢复失败，已恢复原数据: {}", error),
                });
            }
        }
    }

    if let Err(error) = std::fs::remove_file(&marker_path) {
        let rollback_error = rollback_restore_operations(&mut applied).err();
        if rollback_error.is_none() {
            let _ = std::fs::remove_dir_all(&rollback);
        }
        return Err(match rollback_error {
            Some(rollback_error) => format!(
                "提交恢复状态失败: {}; 回滚原数据失败: {}",
                error, rollback_error
            ),
            None => format!("提交恢复状态失败，已恢复原数据: {}", error),
        });
    }
    let _ = std::fs::remove_dir_all(&app_stage);
    Ok(Some(rollback))
}

pub fn apply_pending_restore(app_data: &Path) -> Result<Option<PathBuf>, String> {
    let wiki_base = crate::services::wiki::project::wiki_base_dir();
    apply_pending_restore_with_wiki_base(app_data, &wiki_base, |_| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::{
        apply_pending_restore_with_wiki_base, decrypt_payload, encrypt_payload,
        validate_logical_path, BackupBlob, BackupPayload, BackupSummary, PendingRestore,
    };
    use sha2::Digest;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "crowapi-backup-test-{}-{}",
            label,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        path
    }

    fn write(path: &Path, value: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create test parent");
        }
        std::fs::write(path, value).expect("write test file");
    }

    fn stage_restore(app_data: &Path, wiki_base: &Path, token: &str) {
        let app_stage = app_data.join(format!(".restore-{}", token));
        let wiki_stage = wiki_base
            .parent()
            .expect("wiki parent")
            .join(format!(".restore-{}", token));
        write(&app_stage.join("crowapi.db"), "new-db");
        write(&app_stage.join("kb_files/new.txt"), "new-kb");
        write(&app_stage.join("settings.json"), "new-settings");
        write(&wiki_stage.join("projects/new.md"), "new-wiki");
        write(
            &app_data.join("restore-pending.json"),
            &serde_json::to_string(&PendingRestore {
                token: token.to_string(),
                keep_local_settings: false,
            })
            .expect("serialize pending restore"),
        );
    }

    fn payload() -> BackupPayload {
        let database = b"sqlite-snapshot";
        BackupPayload {
            summary: BackupSummary {
                created_at: "2026-08-20T00:00:00Z".to_string(),
                app_version: "0.1.5".to_string(),
                schema_version: 20,
                database_bytes: database.len() as u64,
                file_count: 0,
                file_bytes: 0,
                channel_count: 1,
                api_key_count: 1,
                knowledge_base_count: 0,
                wiki_project_count: 0,
                includes_logs: false,
            },
            database: BackupBlob {
                sha256: hex::encode(sha2::Sha256::digest(database)),
                data: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    database,
                ),
            },
            files: vec![],
            channel_secrets: HashMap::from([(
                "channel-1".to_string(),
                "sk-secret".to_string(),
            )]),
        }
    }

    #[test]
    fn encrypted_backup_round_trips_and_rejects_wrong_password() {
        let encrypted = encrypt_payload(&payload(), "correct horse battery")
            .expect("encrypt backup");
        let decrypted = decrypt_payload(&encrypted, "correct horse battery")
            .expect("decrypt backup");

        assert_eq!(decrypted.channel_secrets["channel-1"], "sk-secret");
        assert!(decrypt_payload(&encrypted, "incorrect password").is_err());
        assert!(!String::from_utf8_lossy(&encrypted).contains("sk-secret"));
    }

    #[test]
    fn backup_paths_cannot_escape_managed_roots() {
        assert!(validate_logical_path("app/kb_files/doc.txt").is_ok());
        assert!(validate_logical_path("app/settings.json").is_ok());
        assert!(validate_logical_path("wiki/projects/page.md").is_ok());
        assert!(validate_logical_path("app/../outside").is_err());
        assert!(validate_logical_path("app/crowapi.db").is_err());
        assert!(validate_logical_path("app/restore-pending.json").is_err());
        assert!(validate_logical_path("app/kb_files").is_err());
        assert!(validate_logical_path("/absolute/path").is_err());
        assert!(validate_logical_path("other/file").is_err());
    }

    #[test]
    fn pending_restore_replaces_managed_data_and_keeps_rollback() {
        let root = temporary_directory("success");
        let app_data = root.join("app");
        let wiki_base = root.join("data/crowapi/wiki");
        let token = uuid::Uuid::new_v4().to_string();
        write(&app_data.join("crowapi.db"), "old-db");
        write(&app_data.join("crowapi.db-wal"), "old-wal");
        write(&app_data.join("kb_files/old.txt"), "old-kb");
        write(&app_data.join("settings.json"), "old-settings");
        write(&wiki_base.join("projects/old.md"), "old-wiki");
        stage_restore(&app_data, &wiki_base, &token);

        let rollback = apply_pending_restore_with_wiki_base(&app_data, &wiki_base, |_| Ok(()))
            .expect("apply restore")
            .expect("rollback directory");

        assert_eq!(std::fs::read_to_string(app_data.join("crowapi.db")).unwrap(), "new-db");
        assert!(!app_data.join("crowapi.db-wal").exists());
        assert!(app_data.join("kb_files/new.txt").is_file());
        assert!(wiki_base.join("projects/new.md").is_file());
        assert_eq!(
            std::fs::read_to_string(app_data.join("settings.json")).unwrap(),
            "new-settings"
        );
        assert_eq!(std::fs::read_to_string(rollback.join("crowapi.db")).unwrap(), "old-db");
        assert_eq!(std::fs::read_to_string(rollback.join("crowapi.db-wal")).unwrap(), "old-wal");
        assert!(rollback.join("kb_files/old.txt").is_file());
        assert!(rollback.join("wiki/projects/old.md").is_file());
        assert!(!app_data.join("restore-pending.json").exists());

        std::fs::remove_dir_all(root).expect("remove temporary directory");
    }

    #[test]
    fn pending_restore_rolls_back_every_applied_step_on_failure() {
        let root = temporary_directory("rollback");
        let app_data = root.join("app");
        let wiki_base = root.join("data/crowapi/wiki");
        let token = uuid::Uuid::new_v4().to_string();
        write(&app_data.join("crowapi.db"), "old-db");
        write(&app_data.join("kb_files/old.txt"), "old-kb");
        write(&app_data.join("settings.json"), "old-settings");
        write(&wiki_base.join("projects/old.md"), "old-wiki");
        stage_restore(&app_data, &wiki_base, &token);

        let result = apply_pending_restore_with_wiki_base(&app_data, &wiki_base, |step| {
            if step == "wiki-files" {
                Err("injected failure".to_string())
            } else {
                Ok(())
            }
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(app_data.join("crowapi.db")).unwrap(), "old-db");
        assert!(app_data.join("kb_files/old.txt").is_file());
        assert!(wiki_base.join("projects/old.md").is_file());
        assert_eq!(
            std::fs::read_to_string(app_data.join("settings.json")).unwrap(),
            "old-settings"
        );
        assert!(app_data.join("restore-pending.json").is_file());
        assert!(app_data
            .join(format!(".restore-{}/crowapi.db", token))
            .is_file());

        std::fs::remove_dir_all(root).expect("remove temporary directory");
    }
}
