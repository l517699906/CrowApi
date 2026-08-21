use super::{models::KbDocument, safe_path_component};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("Failed to locate application data directory: {}", error))
}

fn kb_storage_dir_from_root(app_data_dir: &Path, kb_id: &str) -> Result<PathBuf, String> {
    safe_path_component(kb_id, "knowledge base ID")?;
    Ok(app_data_dir.join("kb_files").join(kb_id))
}

pub(crate) fn kb_storage_root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("kb_files"))
}

pub fn kb_storage_dir(app: &AppHandle, kb_id: &str) -> Result<PathBuf, String> {
    kb_storage_dir_from_root(&app_data_dir(app)?, kb_id)
}

pub async fn persist_import_snapshot(
    app: &AppHandle,
    kb_id: &str,
    content: &[u8],
) -> Result<PathBuf, String> {
    let directory = kb_storage_dir(app, kb_id)?.join("imports");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("Failed to create import snapshot directory: {}", error))?;
    let path = directory.join(format!("{}.snapshot", uuid::Uuid::new_v4()));
    tokio::fs::write(&path, content)
        .await
        .map_err(|error| format!("Failed to persist import snapshot: {}", error))?;
    Ok(path)
}

async fn remove_managed_file_from_root(
    app_data_dir: &Path,
    kb_id: &str,
    file_path: &Path,
) -> Result<bool, String> {
    let managed_root = kb_storage_dir_from_root(app_data_dir, kb_id)?;
    let canonical_root = match tokio::fs::canonicalize(&managed_root).await {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Failed to resolve managed knowledge base directory: {}",
                error
            ))
        }
    };
    let canonical_file = match tokio::fs::canonicalize(file_path).await {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Failed to resolve managed document file: {}", error)),
    };
    if !canonical_file.starts_with(&canonical_root) {
        return Ok(false);
    }
    tokio::fs::remove_file(file_path)
        .await
        .map_err(|error| format!("Failed to remove managed document file: {}", error))?;
    Ok(true)
}

pub async fn remove_managed_document_file(
    app: &AppHandle,
    kb_id: &str,
    file_path: &Path,
) -> Result<bool, String> {
    remove_managed_file_from_root(&app_data_dir(app)?, kb_id, file_path).await
}

pub async fn cleanup_document_files(app: &AppHandle, documents: &[KbDocument]) {
    for document in documents {
        let Some(file_path) = document.file_path.as_deref() else {
            continue;
        };
        if let Err(error) = remove_managed_document_file(
            app,
            &document.kb_id,
            Path::new(file_path),
        )
        .await
        {
            tracing::warn!(
                %error,
                document_id = %document.id,
                knowledge_base_id = %document.kb_id,
                path = %file_path,
                "failed to clean up managed knowledge document file"
            );
        }
    }
}

async fn remove_kb_storage_from_root(app_data_dir: &Path, kb_id: &str) -> Result<bool, String> {
    let directory = kb_storage_dir_from_root(app_data_dir, kb_id)?;
    match tokio::fs::remove_dir_all(&directory).await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Failed to remove managed knowledge base directory: {}",
            error
        )),
    }
}

pub async fn remove_kb_storage(app: &AppHandle, kb_id: &str) -> Result<bool, String> {
    remove_kb_storage_from_root(&app_data_dir(app)?, kb_id).await
}

#[cfg(test)]
mod tests {
    use super::{remove_kb_storage_from_root, remove_managed_file_from_root};

    #[tokio::test]
    async fn cleanup_is_limited_to_the_managed_knowledge_base_directory() {
        let root = std::env::temp_dir().join(format!("crowapi-kb-storage-{}", uuid::Uuid::new_v4()));
        let kb_id = uuid::Uuid::new_v4().to_string();
        let managed = root.join("kb_files").join(&kb_id).join("imports");
        let managed_file = managed.join("document.snapshot");
        let external_file = root.join("user-document.txt");
        tokio::fs::create_dir_all(&managed).await.unwrap();
        tokio::fs::write(&managed_file, b"managed").await.unwrap();
        tokio::fs::write(&external_file, b"external").await.unwrap();

        assert!(remove_managed_file_from_root(&root, &kb_id, &managed_file)
            .await
            .unwrap());
        assert!(!managed_file.exists());
        assert!(!remove_managed_file_from_root(&root, &kb_id, &external_file)
            .await
            .unwrap());
        assert!(external_file.exists());

        assert!(remove_kb_storage_from_root(&root, &kb_id).await.unwrap());
        assert!(!root.join("kb_files").join(&kb_id).exists());
        tokio::fs::remove_dir_all(&root).await.unwrap();
    }
}
