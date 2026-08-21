use super::WrittenPage;
use std::collections::HashSet;

pub(super) fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut start = 0;
    loop {
        if let Some(s) = content[start..].find("[[") {
            let s = start + s + 2;
            if let Some(e) = content[s..].find("]]") {
                let link = &content[s..s + e];
                if !link.is_empty() {
                    links.push(link.to_string());
                }
                start = s + e + 2;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    links
}

/// Rebuild graph edges for a project based on current page wikilinks.
/// Called after page save/delete to keep the knowledge graph up-to-date.
pub async fn rebuild_graph_edges(
    pool: &sqlx::SqlitePool,
    project_id: &str,
) -> Result<(), String> {
    // Load all pages from DB
    let existing_pages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT path, wikilinks, title FROM wiki_pages WHERE project_id = ? AND status = 'active'"
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let mut all_links: Vec<(String, String)> = Vec::new();

    for (path, wikilinks_json, _title) in &existing_pages {
        let links: Vec<String> = serde_json::from_str(wikilinks_json).unwrap_or_default();
        for link in links {
            let target = resolve_wikilink_to_path(pool, project_id, &link).await;
            // Check if target exists
            if existing_pages.iter().any(|(p, _, _)| p == &target) {
                all_links.push((path.clone(), target));
            }
        }
    }

    replace_graph_edges(pool, project_id, &all_links).await
}

/// Update graph_edges table based on wikilinks in pages.
pub(super) async fn update_graph_edges(
    pool: &sqlx::SqlitePool,
    project_id: &str,
    pages: &[WrittenPage],
) -> Result<(), String> {
    // Collect all valid page paths
    let valid_paths: HashSet<String> = pages.iter().map(|p| p.path.clone()).collect();

    // Also load existing pages from DB
    let existing_pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT path, wikilinks FROM wiki_pages WHERE project_id = ? AND status = 'active'"
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let mut all_links: Vec<(String, String)> = Vec::new();

    // From new pages
    for page in pages {
        for link in &page.wikilinks {
            // Resolve wikilink to actual page path via title matching
            let target = resolve_wikilink_to_path(pool, project_id, link).await;
            if valid_paths.contains(&target) || existing_pages.iter().any(|(p, _)| p == &target) {
                all_links.push((page.path.clone(), target));
            }
        }
    }

    // From existing pages (re-scan to catch new targets)
    for (path, wikilinks_json) in &existing_pages {
        let links: Vec<String> = serde_json::from_str(wikilinks_json).unwrap_or_default();
        for link in links {
            let target = resolve_wikilink_to_path(pool, project_id, &link).await;
            // Check if target exists (in new pages or existing)
            if valid_paths.contains(&target) || existing_pages.iter().any(|(p, _)| p == &target) {
                all_links.push((path.clone(), target));
            }
        }
    }

    replace_graph_edges(pool, project_id, &all_links).await
}

pub(super) async fn replace_graph_edges(
    pool: &sqlx::SqlitePool,
    project_id: &str,
    links: &[(String, String)],
) -> Result<(), String> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    sqlx::query("DELETE FROM wiki_graph_edges WHERE project_id = ?")
        .bind(project_id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();
    for (source, target) in links {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT OR IGNORE INTO wiki_graph_edges (id, project_id, source_page, target_page, edge_type, weight, created_at)
             VALUES (?, ?, ?, ?, 'wikilink', 1.0, ?)"
        )
        .bind(&id)
        .bind(project_id)
        .bind(source)
        .bind(target)
        .bind(&now)
        .execute(&mut *transaction)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    }

    transaction
        .commit()
        .await
        .map_err(|e| format!("DB error: {}", e))
}

/// Extract tags from YAML frontmatter of a wiki page.
pub fn extract_tags_from_frontmatter(content: &str) -> Vec<String> {
    let Some(frontmatter) = extract_frontmatter(content) else {
        return vec![];
    };
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("tags:") {
            let tags_part = trimmed[5..].trim();
            // Parse array format: [tag1, tag2] or ["tag1", "tag2"]
            let cleaned = tags_part
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim();
            if cleaned.is_empty() {
                return vec![];
            }
            return cleaned
                .split(',')
                .map(|t| t.trim().trim_matches('"').trim_matches('\'').trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }
    }
    vec![]
}

/// Return the YAML frontmatter exactly as stored in the Markdown page.
pub fn extract_frontmatter(content: &str) -> Option<&str> {
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;
    let end = rest
        .find("\n---\n")
        .or_else(|| rest.find("\r\n---\r\n"))
        .or_else(|| rest.strip_suffix("\n---").map(|value| value.len()))
        .or_else(|| rest.strip_suffix("\r\n---").map(|value| value.len()))?;
    Some(rest[..end].trim())
}

fn normalize_wikilink(link: &str) -> String {
    let link = link.trim();
    // If it already looks like a path, use as-is
    if link.contains('/') && link.ends_with(".md") {
        return link.to_string();
    }
    // Otherwise, assume it's an entity name
    let slug: String = link.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    format!("entities/{}.md", slug)
}

/// Resolve a wikilink to an actual page path by matching against known page titles.
/// Falls back to `normalize_wikilink` if no title match is found.
async fn resolve_wikilink_to_path(
    pool: &sqlx::SqlitePool,
    project_id: &str,
    link: &str,
) -> String {
    let link = link.trim();
    // If it already looks like a path, use as-is
    if link.contains('/') && link.ends_with(".md") {
        return link.to_string();
    }
    // Try exact title match (case-insensitive)
    let title_match: Option<(String,)> = sqlx::query_as(
        "SELECT path FROM wiki_pages WHERE project_id = ? AND status = 'active' AND LOWER(title) = LOWER(?) LIMIT 1"
    )
    .bind(project_id)
    .bind(link)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if let Some((path,)) = title_match {
        return path;
    }
    // Fallback to slug-based normalization
    normalize_wikilink(link)
}

