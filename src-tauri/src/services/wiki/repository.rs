use super::models::*;
use sqlx::SqlitePool;
use uuid::Uuid;
use chrono::Utc;

fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn content_snippet(content: &str, query: &str) -> String {
    let lower_content = content.to_lowercase();
    let lower_query = query.to_lowercase();
    let position = if lower_query.is_empty() {
        None
    } else {
        lower_content.find(&lower_query)
    };
    let start_hint = position.unwrap_or(0).min(content.len());
    let start = content.floor_char_boundary(start_hint.saturating_sub(60));
    let end = content.ceil_char_boundary(start.saturating_add(200).min(content.len()));
    let snippet = content[start..end].replace('\n', " ");
    if position.is_some() {
        format!("...{}...", snippet)
    } else {
        snippet
    }
}

pub struct WikiRepository {
    pool: SqlitePool,
}

impl WikiRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn now() -> String {
        Utc::now().to_rfc3339()
    }

    fn uuid() -> String {
        Uuid::new_v4().to_string()
    }

    // ── Project CRUD ──

    pub async fn list_projects(&self) -> Result<Vec<WikiProject>, String> {
        sqlx::query_as::<_, WikiProject>(
            "SELECT * FROM wiki_projects ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn get_project(&self, id: &str) -> Result<WikiProject, String> {
        sqlx::query_as::<_, WikiProject>("SELECT * FROM wiki_projects WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn find_project(&self, id: &str) -> Result<Option<WikiProject>, String> {
        sqlx::query_as::<_, WikiProject>("SELECT * FROM wiki_projects WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn create_project(&self, input: &CreateProjectInput, wiki_dir: &str) -> Result<WikiProject, String> {
        let id = Self::uuid();
        self.create_project_with_id(&id, input, wiki_dir).await
    }

    pub async fn create_project_with_id(&self, id: &str, input: &CreateProjectInput, wiki_dir: &str) -> Result<WikiProject, String> {
        let now = Self::now();
        let schema = input.schema_text.clone().unwrap_or_else(|| DEFAULT_SCHEMA.to_string());

        sqlx::query(
            "INSERT INTO wiki_projects (id, name, description, status, schema_text, wiki_dir, ingest_model, chat_model, ingest_channel_id, chat_channel_id, mcp_enabled, source_count, page_count, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?, ?, ?, ?, ?, 1, 0, 0, ?, ?)"
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&schema)
        .bind(wiki_dir)
        .bind(&input.ingest_model)
        .bind(&input.chat_model)
        .bind(&input.ingest_channel_id)
        .bind(&input.chat_channel_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        self.get_project(&id).await
    }

    pub async fn update_project(&self, id: &str, input: &UpdateProjectInput) -> Result<WikiProject, String> {
        let now = Self::now();
        let current = self.get_project(id).await?;

        let name = input.name.clone().unwrap_or(current.name);
        let description = input.description.clone().or(current.description);
        let status = input.status.unwrap_or(current.status);
        let schema_text = input.schema_text.clone().or(current.schema_text);
        let ingest_model = input.ingest_model.clone().or(current.ingest_model);
        let chat_model = input.chat_model.clone().or(current.chat_model);
        let ingest_channel_id = input.ingest_channel_id.clone().or(current.ingest_channel_id);
        let chat_channel_id = input.chat_channel_id.clone().or(current.chat_channel_id);
        let mcp_enabled = input.mcp_enabled.unwrap_or(current.mcp_enabled);

        sqlx::query(
            "UPDATE wiki_projects SET name=?, description=?, status=?, schema_text=?, ingest_model=?, chat_model=?, ingest_channel_id=?, chat_channel_id=?, mcp_enabled=?, updated_at=? WHERE id=?"
        )
        .bind(&name)
        .bind(&description)
        .bind(status)
        .bind(&schema_text)
        .bind(&ingest_model)
        .bind(&chat_model)
        .bind(&ingest_channel_id)
        .bind(&chat_channel_id)
        .bind(mcp_enabled)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        self.get_project(id).await
    }

    pub async fn delete_project(&self, id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM wiki_projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    // ── Pages ──

    pub async fn list_pages(&self, project_id: &str) -> Result<Vec<WikiPage>, String> {
        sqlx::query_as::<_, WikiPage>(
            "SELECT * FROM wiki_pages WHERE project_id = ? AND status = 'active' ORDER BY path"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn get_page(&self, project_id: &str, path: &str) -> Result<Option<WikiPage>, String> {
        sqlx::query_as::<_, WikiPage>(
            "SELECT * FROM wiki_pages WHERE project_id = ? AND path = ?"
        )
        .bind(project_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn upsert_page(
        &self,
        project_id: &str,
        path: &str,
        title: &str,
        page_type: &str,
        content_hash: &str,
        token_count: i64,
        wikilinks: &str,
        frontmatter: &str,
        tags: &str,
    ) -> Result<(), String> {
        let now = Self::now();
        sqlx::query(
            "INSERT INTO wiki_pages (id, project_id, path, title, page_type, content_hash, token_count, wikilinks, frontmatter, tags, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
             ON CONFLICT(project_id, path) DO UPDATE SET
               title=excluded.title, page_type=excluded.page_type,
               content_hash=excluded.content_hash, token_count=excluded.token_count,
               wikilinks=excluded.wikilinks, frontmatter=excluded.frontmatter,
               tags=excluded.tags, status='active', updated_at=excluded.updated_at"
        )
        .bind(Self::uuid())
        .bind(project_id)
        .bind(path)
        .bind(title)
        .bind(page_type)
        .bind(content_hash)
        .bind(token_count)
        .bind(wikilinks)
        .bind(frontmatter)
        .bind(tags)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    pub async fn delete_page(&self, project_id: &str, path: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM wiki_pages WHERE project_id = ? AND path = ?")
            .bind(project_id)
            .bind(path)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    pub async fn search_pages(&self, project_id: &str, query: &str, top_k: usize) -> Result<Vec<WikiSearchResult>, String> {
        // The current schema has no FTS table. Escape wildcard characters so
        // literal searches cannot broaden the result set unexpectedly.
        let pattern = format!("%{}%", escape_like_pattern(query));

        // First try LIKE on title and path for quick matches
        let like_rows = sqlx::query_as::<_, WikiPage>(
            "SELECT * FROM wiki_pages WHERE project_id = ? AND status = 'active'
             AND (title LIKE ? ESCAPE '\\' OR path LIKE ? ESCAPE '\\')
             ORDER BY title LIMIT ?"
        )
        .bind(project_id)
        .bind(&pattern)
        .bind(&pattern)
        .bind(top_k as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        // Also search in file contents on disk for better recall
        let mut results: Vec<WikiSearchResult> = like_rows.iter().map(|p| {
            WikiSearchResult {
                page_id: p.id.clone(),
                path: p.path.clone(),
                title: p.title.clone(),
                score: 1.0,
                snippet: String::new(),
                page_type: p.page_type.clone(),
            }
        }).collect();

        // If we have fewer results than top_k, try reading page files and searching content
        if results.len() < top_k {
            let all_pages = sqlx::query_as::<_, WikiPage>(
                "SELECT * FROM wiki_pages WHERE project_id = ? AND status = 'active' ORDER BY title"
            )
            .bind(project_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;

            let existing_paths: std::collections::HashSet<String> = results.iter().map(|r| r.path.clone()).collect();

            for page in &all_pages {
                if existing_paths.contains(&page.path) {
                    continue;
                }
                // Read content from disk and search
                if let Ok(content) = crate::services::wiki::project::read_page(project_id, &page.path).await {
                    let content_lower = content.to_lowercase();
                    let query_lower = query.to_lowercase();
                    if content_lower.contains(&query_lower) {
                        let snippet = content_snippet(&content, query);

                        results.push(WikiSearchResult {
                            page_id: page.id.clone(),
                            path: page.path.clone(),
                            title: page.title.clone(),
                            score: 0.8,
                            snippet,
                            page_type: page.page_type.clone(),
                        });

                        if results.len() >= top_k {
                            break;
                        }
                    }
                }
            }
        } else {
            // Add snippets for LIKE matches too
            for r in &mut results {
                if let Ok(content) = crate::services::wiki::project::read_page(project_id, &r.path).await {
                    r.snippet = content_snippet(&content, query);
                }
            }
        }

        Ok(results)
    }

    // ── Sources ──

    pub async fn list_sources(&self, project_id: &str) -> Result<Vec<WikiSource>, String> {
        sqlx::query_as::<_, WikiSource>(
            "SELECT * FROM wiki_sources WHERE project_id = ? ORDER BY created_at DESC"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn get_source(&self, source_id: &str) -> Result<WikiSource, String> {
        sqlx::query_as::<_, WikiSource>(
            "SELECT * FROM wiki_sources WHERE id = ?"
        )
        .bind(source_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn add_source(&self, project_id: &str, input: &AddSourceInput, content_hash: Option<&str>, file_size: i64) -> Result<WikiSource, String> {
        let id = Self::uuid();
        let now = Self::now();
        sqlx::query(
            "INSERT INTO wiki_sources (id, project_id, source_type, filename, file_path, source_url, content_hash, file_size, status, page_count, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', 0, ?)"
        )
        .bind(&id)
        .bind(project_id)
        .bind(&input.source_type)
        .bind(&input.filename)
        .bind(&input.file_path)
        .bind(&input.source_url)
        .bind(content_hash)
        .bind(file_size)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        sqlx::query_as::<_, WikiSource>("SELECT * FROM wiki_sources WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn find_source(&self, source_id: &str) -> Result<Option<WikiSource>, String> {
        sqlx::query_as::<_, WikiSource>(
            "SELECT * FROM wiki_sources WHERE id = ?"
        )
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn delete_source(&self, source_id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM wiki_sources WHERE id = ?")
            .bind(source_id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    pub async fn update_source_status(&self, source_id: &str, status: &str, page_count: i64, error: Option<&str>) -> Result<(), String> {
        let now = Self::now();
        sqlx::query(
            "UPDATE wiki_sources SET status=?, page_count=?, error_message=?, ingested_at=? WHERE id=?"
        )
        .bind(status)
        .bind(page_count)
        .bind(error)
        .bind(if status == "ingested" { Some(now.clone()) } else { None })
        .bind(source_id)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    // ── Ingest Queue ──

    pub async fn create_task_if_idle(
        &self,
        project_id: &str,
        source_id: &str,
        task_type: &str,
    ) -> Result<Option<String>, String> {
        let id = Self::uuid();
        let now = Self::now();
        let inserted = sqlx::query(
            "INSERT INTO wiki_ingest_queue
             (id, project_id, source_id, task_type, status, progress, total_steps, done_steps, created_at)
             SELECT ?, ?, ?, ?, 'pending', 0, 0, 0, ?
             WHERE NOT EXISTS (
                 SELECT 1 FROM wiki_ingest_queue
                 WHERE project_id = ? AND source_id = ? AND task_type = ?
                   AND status IN ('pending', 'running')
             )",
        )
        .bind(&id)
        .bind(project_id)
        .bind(source_id)
        .bind(task_type)
        .bind(&now)
        .bind(project_id)
        .bind(source_id)
        .bind(task_type)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        Ok((inserted.rows_affected() == 1).then_some(id))
    }

    pub async fn update_task_status(&self, task_id: &str, status: &str, progress: i64, done_steps: i64, total_steps: i64, result: Option<&str>, error: Option<&str>) -> Result<(), String> {
        let now = Self::now();
        let started = if status == "running" { Some(now.clone()) } else { None };
        let completed = if status == "done" || status == "failed" { Some(now.clone()) } else { None };
        sqlx::query(
            "UPDATE wiki_ingest_queue SET status=?, progress=?, done_steps=?, total_steps=?, result_json=?, error_message=?, started_at=COALESCE(?, started_at), completed_at=COALESCE(?, completed_at) WHERE id=?"
        )
        .bind(status)
        .bind(progress)
        .bind(done_steps)
        .bind(total_steps)
        .bind(result)
        .bind(error)
        .bind(started.as_deref())
        .bind(completed.as_deref())
        .bind(task_id)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    pub async fn list_tasks(&self, project_id: &str) -> Result<Vec<WikiIngestTask>, String> {
        sqlx::query_as::<_, WikiIngestTask>(
            "SELECT * FROM wiki_ingest_queue WHERE project_id = ? ORDER BY created_at DESC LIMIT 20"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    // ── Sessions ──

    pub async fn list_sessions(&self, project_id: &str) -> Result<Vec<WikiSession>, String> {
        sqlx::query_as::<_, WikiSession>(
            "SELECT * FROM wiki_sessions WHERE project_id = ? ORDER BY created_at DESC LIMIT 50"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn add_session(&self, project_id: &str, role: &str, content: &str, sources: Option<&str>, model: Option<&str>) -> Result<(), String> {
        let id = Self::uuid();
        let now = Self::now();
        sqlx::query(
            "INSERT INTO wiki_sessions (id, project_id, role, content, sources_json, model, tokens_used, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 0, ?)"
        )
        .bind(&id)
        .bind(project_id)
        .bind(role)
        .bind(content)
        .bind(sources)
        .bind(model)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    pub async fn clear_sessions(&self, project_id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM wiki_sessions WHERE project_id = ?")
            .bind(project_id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    // ── Tags ──

    pub async fn get_tags(&self, project_id: &str, limit: usize) -> Result<Vec<crate::services::wiki::models::WikiTag>, String> {
        // Collect tags from all active pages in the project
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT tags FROM wiki_pages WHERE project_id = ? AND status = 'active'"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let mut freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (tags_json,) in &rows {
            let tags: Vec<String> = serde_json::from_str(tags_json).unwrap_or_default();
            for tag in tags {
                let tag = tag.trim().to_lowercase();
                if !tag.is_empty() {
                    *freq.entry(tag).or_insert(0) += 1;
                }
            }
        }

        let mut freq_vec: Vec<(String, usize)> = freq.into_iter().collect();
        freq_vec.sort_by(|a, b| b.1.cmp(&a.1));

        Ok(freq_vec
            .into_iter()
            .take(limit)
            .map(|(word, count)| crate::services::wiki::models::WikiTag { word, count })
            .collect())
    }

    // ── Graph ──

    pub async fn get_graph(&self, project_id: &str) -> Result<GraphData, String> {
        let pages = self.list_pages(project_id).await?;

        let nodes: Vec<GraphNode> = pages.iter().map(|p| {
            let links: Vec<String> = serde_json::from_str(&p.wikilinks).unwrap_or_default();
            GraphNode {
                id: p.path.clone(),
                label: p.title.clone(),
                path: Some(p.path.clone()),
                node_type: p.page_type.clone(),
                link_count: links.len(),
            }
        }).collect();

        let edges_rows = sqlx::query_as::<_, WikiGraphEdgeRow>(
            "SELECT source_page, target_page, edge_type, weight FROM wiki_graph_edges WHERE project_id = ? ORDER BY weight DESC"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let edges: Vec<GraphEdge> = edges_rows.iter().map(|r| GraphEdge {
            source: r.source_page.clone(),
            target: r.target_page.clone(),
            edge_type: r.edge_type.clone(),
            weight: r.weight,
        }).collect();

        Ok(GraphData { nodes, edges })
    }

    // ── Stats ──

    pub async fn get_stats(&self, project_id: &str) -> Result<serde_json::Value, String> {
        let page_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM wiki_pages WHERE project_id = ? AND status = 'active'"
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let source_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM wiki_sources WHERE project_id = ? AND status = 'ingested'"
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let review_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM wiki_reviews WHERE project_id = ? AND resolved = 0"
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let page_types: Vec<(String, i64)> = sqlx::query_as(
            "SELECT page_type, COUNT(*) as cnt FROM wiki_pages WHERE project_id = ? AND status = 'active' GROUP BY page_type"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let type_map: serde_json::Map<String, serde_json::Value> = page_types.into_iter()
            .map(|(t, c)| (t, serde_json::Value::Number(c.into())))
            .collect();

        Ok(serde_json::json!({
            "pages": page_count,
            "sources": source_count,
            "pending_reviews": review_count,
            "page_types": serde_json::Value::Object(type_map),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{content_snippet, escape_like_pattern, WikiRepository};
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn like_patterns_escape_sql_wildcards() {
        assert_eq!(escape_like_pattern("50%_done\\"), "50\\%\\_done\\\\");
    }

    #[test]
    fn snippets_preserve_utf8_boundaries() {
        let content = "前置内容 ".repeat(40) + "目标词" + &" 后续内容".repeat(40);
        let snippet = content_snippet(&content, "目标词");
        assert!(snippet.contains("目标词"));
        assert!(std::str::from_utf8(snippet.as_bytes()).is_ok());
    }

    #[tokio::test]
    async fn ingest_task_claim_is_atomic_for_a_source() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create Wiki repository test database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        let now = "2026-08-20T00:00:00Z";
        sqlx::query(
            "INSERT INTO wiki_projects
             (id, name, status, wiki_dir, mcp_enabled, source_count, page_count, created_at, updated_at)
             VALUES ('project', 'test', 1, '/tmp/wiki', 1, 0, 0, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert project");
        sqlx::query(
            "INSERT INTO wiki_sources
             (id, project_id, source_type, filename, file_size, status, page_count, created_at)
             VALUES ('source', 'project', 'upload', 'source.md', 0, 'pending', 0, ?)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert source");

        let repo = WikiRepository::new(pool);
        let first = repo
            .create_task_if_idle("project", "source", "ingest")
            .await
            .expect("claim first ingest")
            .expect("first ingest is claimed");
        assert!(repo
            .create_task_if_idle("project", "source", "ingest")
            .await
            .expect("attempt duplicate ingest")
            .is_none());
        repo.update_task_status(&first, "failed", 0, 0, 3, None, Some("failed"))
            .await
            .expect("finish first ingest");
        assert!(repo
            .create_task_if_idle("project", "source", "ingest")
            .await
            .expect("claim retry ingest")
            .is_some());
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct WikiGraphEdgeRow {
    source_page: String,
    target_page: String,
    edge_type: String,
    weight: f64,
}

pub const DEFAULT_SCHEMA: &str = r#"# Wiki Schema

You are a Wiki maintainer. Your job is to read source documents and maintain a structured, interlinked wiki.

## Rules
- Each page should focus on one entity, concept, or topic.
- Use `[[wikilinks]]` to connect related pages.
- Update `index.md` when adding or removing pages.
- Append to `log.md` on every ingest.
- Flag contradictions, missing pages, and stale information as reviews.
- Use YAML frontmatter on each page (title, type, tags, date, source_count).

## Page Types
- `entity`: A specific thing (person, project, tool, module).
- `concept`: An abstract idea or pattern.
- `summary`: A condensed overview of a source.
- `review`: A flagged item needing human attention.

## File Layout
- `wiki/index.md` — content catalog.
- `wiki/log.md` — chronological operation log.
- `wiki/entities/*.md` — entity pages.
- `wiki/concepts/*.md` — concept pages.
- `wiki/summaries/*.md` — source summaries.
"#;
