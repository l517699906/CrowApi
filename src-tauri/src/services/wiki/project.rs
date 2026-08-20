use std::path::{Component, Path, PathBuf};
use tokio::fs;
use uuid::Uuid;
use chrono::Utc;

/// Get the base wiki directory for all projects.
pub fn wiki_base_dir() -> PathBuf {
    let data_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("./data"));
    let current = data_dir.join("crowapi").join("wiki");
    let legacy = data_dir.join("waliapi").join("wiki");
    let base = if !current.exists() && legacy.exists() {
        legacy
    } else {
        current
    };
    if let Err(error) = std::fs::create_dir_all(&base) {
        tracing::warn!(%error, path = %base.display(), "failed to create Wiki base directory");
    }
    base
}

fn safe_path_component<'a>(value: &'a str, label: &str) -> Result<&'a str, String> {
    if value.contains('\\') {
        return Err(format!("Invalid {}", label));
    }
    let mut components = Path::new(value).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) if !value.is_empty() => Ok(value),
        _ => Err(format!("Invalid {}", label)),
    }
}

fn safe_relative_path(value: &str, label: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.contains('\\') {
        return Err(format!("Invalid {}", label));
    }

    let mut safe = PathBuf::new();
    for component in Path::new(value).components() {
        match component {
            Component::Normal(part) => safe.push(part),
            _ => return Err(format!("Invalid {}", label)),
        }
    }

    if safe.as_os_str().is_empty() {
        return Err(format!("Invalid {}", label));
    }
    Ok(safe)
}

/// Get a project's wiki directory.
pub fn project_wiki_dir(project_id: &str) -> Result<PathBuf, String> {
    let dir = project_dir_path(project_id)?;
    let directories = [
        dir.clone(),
        dir.join("raw").join("sources"),
        dir.join("raw").join("assets"),
        dir.join("wiki").join("entities"),
        dir.join("wiki").join("concepts"),
        dir.join("wiki").join("summaries"),
        dir.join("schema"),
    ];
    for path in directories {
        if let Err(error) = std::fs::create_dir_all(&path) {
            return Err(format!("Failed to create Wiki project directory {}: {}", path.display(), error));
        }
    }
    Ok(dir)
}

fn project_dir_path(project_id: &str) -> Result<PathBuf, String> {
    safe_path_component(project_id, "project id")?;
    Ok(wiki_base_dir().join("projects").join(project_id))
}

pub struct StagedRemoval {
    original: PathBuf,
    staged: PathBuf,
}

async fn stage_removal(path: PathBuf) -> Result<Option<StagedRemoval>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let parent = path.parent().ok_or_else(|| "Invalid removal path".to_string())?;
    let staged = parent.join(format!(".crowapi-delete-{}", Uuid::new_v4()));
    fs::rename(&path, &staged).await
        .map_err(|e| format!("Failed to stage {} for deletion: {}", path.display(), e))?;
    Ok(Some(StagedRemoval { original: path, staged }))
}

pub async fn stage_project_dir_removal(project_id: &str) -> Result<Option<StagedRemoval>, String> {
    stage_removal(project_dir_path(project_id)?).await
}

pub async fn stage_page_file_removal(project_id: &str, path: &str) -> Result<Option<StagedRemoval>, String> {
    let file_path = project_wiki_dir(project_id)?
        .join("wiki")
        .join(safe_relative_path(path, "page path")?);
    stage_removal(file_path).await
}

/// Return the managed path used for a source uploaded through the Wiki UI.
/// External file paths are intentionally kept outside this helper so deleting a
/// source never removes a user's original file.
pub fn source_file_path(project_id: &str, filename: &str) -> Result<PathBuf, String> {
    let dir = project_wiki_dir(project_id)?;
    let filename = safe_path_component(filename, "source filename")?;
    Ok(dir.join("raw").join("sources").join(filename))
}

/// Stage a managed source file for deletion. A persisted external `file_path`
/// is ignored; only files inside the project's raw/sources directory are owned
/// by CrowAPI and may be removed.
pub async fn stage_source_file_removal(
    project_id: &str,
    filename: &str,
    file_path: Option<&str>,
) -> Result<Option<StagedRemoval>, String> {
    let managed_dir = project_wiki_dir(project_id)?.join("raw").join("sources");
    let managed_path = file_path
        .map(PathBuf::from)
        .filter(|path| path.starts_with(&managed_dir))
        .unwrap_or(source_file_path(project_id, filename)?);
    stage_removal(managed_path).await
}

pub async fn restore_staged_removal(removal: &StagedRemoval) -> Result<(), String> {
    fs::rename(&removal.staged, &removal.original).await
        .map_err(|e| format!("Failed to restore {}: {}", removal.original.display(), e))
}

pub async fn finalize_staged_removal(removal: StagedRemoval) -> Result<(), String> {
    let metadata = match fs::metadata(&removal.staged).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Failed to inspect staged removal: {}", error)),
    };
    if metadata.is_dir() {
        fs::remove_dir_all(&removal.staged).await
            .map_err(|e| format!("Failed to remove staged directory: {}", e))
    } else {
        fs::remove_file(&removal.staged).await
            .map_err(|e| format!("Failed to remove staged file: {}", e))
    }
}

/// Initialize a new wiki project directory structure.
pub async fn init_project_dir(project_id: &str, schema_text: &str) -> Result<PathBuf, String> {
    let dir = project_wiki_dir(project_id)?;

    // Write schema/CLAUDE.md
    let schema_path = dir.join("schema").join("CLAUDE.md");
    fs::write(&schema_path, schema_text).await
        .map_err(|e| format!("Failed to write schema: {}", e))?;

    // Write wiki/index.md
    let index_path = dir.join("wiki").join("index.md");
    if !index_path.exists() {
        fs::write(&index_path, "# Wiki Index\n\n<!-- Add pages below -->\n").await
            .map_err(|e| format!("Failed to write index: {}", e))?;
    }

    // Write wiki/log.md
    let log_path = dir.join("wiki").join("log.md");
    if !log_path.exists() {
        let now = Utc::now().to_rfc3339();
        fs::write(&log_path, &format!("# Wiki Log\n\n## [{}] init | Project created\n", now)).await
            .map_err(|e| format!("Failed to write log: {}", e))?;
    }

    // Write .meta.json
    let meta_path = dir.join(".meta.json");
    let meta = serde_json::json!({
        "project_id": project_id,
        "created_at": Utc::now().to_rfc3339(),
    });
    fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap()).await
        .map_err(|e| format!("Failed to write meta: {}", e))?;

    Ok(dir)
}

/// Read a wiki page from disk.
pub async fn read_page(project_id: &str, path: &str) -> Result<String, String> {
    let dir = project_wiki_dir(project_id)?;
    let full_path = dir.join("wiki").join(safe_relative_path(path, "page path")?);
    fs::read_to_string(&full_path).await
        .map_err(|e| format!("Failed to read {}: {}", path, e))
}

pub async fn snapshot_page(project_id: &str, path: &str) -> Result<Option<String>, String> {
    let dir = project_wiki_dir(project_id)?;
    let full_path = dir.join("wiki").join(safe_relative_path(path, "page path")?);
    match fs::read_to_string(&full_path).await {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Failed to read {}: {}", path, error)),
    }
}

pub async fn rollback_page(project_id: &str, path: &str, previous: Option<&str>) -> Result<(), String> {
    match previous {
        Some(content) => write_page(project_id, path, content).await,
        None => delete_page_file(project_id, path).await,
    }
}

/// Write a wiki page to disk.
pub async fn write_page(project_id: &str, path: &str, content: &str) -> Result<(), String> {
    let dir = project_wiki_dir(project_id)?;
    let full_path = dir.join("wiki").join(safe_relative_path(path, "page path")?);
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).await
            .map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    fs::write(&full_path, content).await
        .map_err(|e| format!("Failed to write {}: {}", path, e))
}

/// Delete a wiki page from disk.
pub async fn delete_page_file(project_id: &str, path: &str) -> Result<(), String> {
    let dir = project_wiki_dir(project_id)?;
    let full_path = dir.join("wiki").join(safe_relative_path(path, "page path")?);
    if full_path.exists() {
        fs::remove_file(&full_path).await
            .map_err(|e| format!("Failed to delete {}: {}", path, e))?;
    }
    Ok(())
}

/// Append to log.md
pub async fn append_log(project_id: &str, entry: &str) -> Result<(), String> {
    let dir = project_wiki_dir(project_id)?;
    let log_path = dir.join("wiki").join("log.md");
    let now = Utc::now().format("%Y-%m-%d %H:%M").to_string();
    let line = format!("\n## [{}] {}\n", now, entry);
    let mut content = match fs::read_to_string(&log_path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "# Wiki Log\n".to_string(),
        Err(error) => return Err(format!("Failed to read Wiki log: {}", error)),
    };
    content.push_str(&line);
    fs::write(&log_path, &content).await
        .map_err(|e| format!("Failed to append log: {}", e))
}

/// Update index.md with a new entry.
pub async fn update_index(project_id: &str, entries: &[IndexEntry]) -> Result<(), String> {
    let dir = project_wiki_dir(project_id)?;
    let index_path = dir.join("wiki").join("index.md");
    let mut content = String::from("# Wiki Index\n\n");
    for entry in entries {
        content.push_str(&format!("- [[{}]] — {}\n", entry.path, entry.summary));
    }
    fs::write(&index_path, &content).await
        .map_err(|e| format!("Failed to write index: {}", e))
}

pub struct IndexEntry {
    pub path: String,
    pub summary: String,
}

/// List all wiki page files on disk.
pub async fn list_page_files(project_id: &str) -> Result<Vec<PageFileInfo>, String> {
    let dir = project_wiki_dir(project_id)?;
    let wiki_dir = dir.join("wiki");
    let mut results = Vec::new();
    let mut stack = vec![wiki_dir.clone()];
    while let Some(current) = stack.pop() {
        let mut entries = match fs::read_dir(&current).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(ext) = path.extension() {
                if ext == "md" {
                    let rel_path = path.strip_prefix(&wiki_dir).unwrap_or(&path).to_string_lossy().to_string();
                    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    results.push(PageFileInfo { path: rel_path, title: name });
                }
            }
        }
    }
    Ok(results)
}

pub struct PageFileInfo {
    pub path: String,
    pub title: String,
}

/// Write a source file to raw/sources/.
pub async fn write_source_file(project_id: &str, filename: &str, content: &[u8]) -> Result<PathBuf, String> {
    let dir = project_wiki_dir(project_id)?;
    let sources_dir = dir.join("raw").join("sources");
    fs::create_dir_all(&sources_dir).await
        .map_err(|e| format!("Failed to create sources dir: {}", e))?;
    let file_path = source_file_path(project_id, filename)?;
    fs::write(&file_path, content).await
        .map_err(|e| format!("Failed to write source file: {}", e))?;
    Ok(file_path)
}

/// Read a source file from raw/sources/ or raw/.
pub async fn read_source_file(project_id: &str, path: &str) -> Result<Vec<u8>, String> {
    let dir = project_wiki_dir(project_id)?;
    let relative = safe_relative_path(path, "source path")?;
    let safe_path = if relative.starts_with(Path::new("raw/sources"))
        || relative.starts_with(Path::new("wiki"))
    {
        dir.join(relative)
    } else {
        dir.join("raw").join("sources").join(relative)
    };

    fs::read(&safe_path).await
        .map_err(|e| format!("Failed to read file: {}", e))
}

/// Remove a project directory entirely.
pub async fn remove_project_dir(project_id: &str) -> Result<(), String> {
    let dir = project_dir_path(project_id)?;
    if dir.exists() {
        fs::remove_dir_all(&dir).await
            .map_err(|e| format!("Failed to remove project dir: {}", e))?;
    }
    Ok(())
}

pub fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::{safe_path_component, safe_relative_path};

    #[test]
    fn accepts_normal_wiki_paths() {
        assert_eq!(
            safe_relative_path("concepts/routing.md", "page path").unwrap(),
            std::path::PathBuf::from("concepts/routing.md"),
        );
        assert!(safe_path_component("project-123", "project id").is_ok());
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        for path in ["../outside.md", "concepts/../../outside.md", "/tmp/outside.md", r"..\\outside.md", "."] {
            assert!(safe_relative_path(path, "page path").is_err(), "accepted unsafe path: {path}");
        }
        for value in ["../project", "nested/project", r"..\\project", "", "."] {
            assert!(safe_path_component(value, "project id").is_err(), "accepted unsafe component: {value}");
        }
    }
}
