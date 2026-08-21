use super::models::*;
use sqlx::SqlitePool;
use uuid::Uuid;
use chrono::Utc;
use crate::services::tasks::{
    models::{BackgroundTask, TaskListFilter, TaskSpec},
    repository::TaskRepository,
};

fn background_to_wiki_task(task: BackgroundTask) -> WikiIngestTask {
    WikiIngestTask {
        id: task.id,
        project_id: task.resource_id,
        source_id: task.subject_id,
        task_type: task.task_type,
        status: if task.status == "succeeded" {
            "done".to_string()
        } else {
            task.status
        },
        progress: task.progress,
        total_steps: task.total_items,
        done_steps: task.done_items,
        result_json: task.result_json,
        error_message: task.error_message,
        created_at: task.created_at,
        started_at: task.started_at,
        completed_at: task.completed_at,
    }
}

/// Convert user input into a literal FTS5 expression.  Quoting each term
/// keeps punctuation and operators (`OR`, `NOT`, `*`, etc.) from changing the
/// query semantics while still allowing multi-word searches to be combined.
fn fts_query(value: &str) -> Option<String> {
    let terms: Vec<String> = value
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

const MAX_SEARCH_LIMIT: usize = 100;

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
        let mut tx = self.pool.begin().await.map_err(|e| format!("DB error: {}", e))?;
        // FTS5 tables do not participate in the Wiki foreign-key cascade, so
        // remove their projection explicitly before deleting the project.
        sqlx::query("DELETE FROM wiki_page_search WHERE project_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        sqlx::query("DELETE FROM wiki_projects WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        tx.commit().await.map_err(|e| format!("DB error: {}", e))?;
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
        content: &str,
    ) -> Result<(), String> {
        let now = Self::now();
        let mut tx = self.pool.begin().await.map_err(|e| format!("DB error: {}", e))?;
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
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let page_id: String = sqlx::query_scalar(
            "SELECT id FROM wiki_pages WHERE project_id = ? AND path = ?",
        )
        .bind(project_id)
        .bind(path)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        sqlx::query("DELETE FROM wiki_page_search WHERE page_id = ?")
            .bind(&page_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        sqlx::query(
            "INSERT INTO wiki_page_search (page_id, project_id, path, title, content, tags)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&page_id)
        .bind(project_id)
        .bind(path)
        .bind(title)
        .bind(content)
        .bind(tags)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        tx.commit().await.map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    pub async fn delete_page(&self, project_id: &str, path: &str) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(|e| format!("DB error: {}", e))?;
        sqlx::query("DELETE FROM wiki_page_search WHERE project_id = ? AND path = ?")
            .bind(project_id)
            .bind(path)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        sqlx::query("DELETE FROM wiki_pages WHERE project_id = ? AND path = ?")
            .bind(project_id)
            .bind(path)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        tx.commit().await.map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    /// Preserve the original item-only API while using the paged FTS query.
    pub async fn search_pages(
        &self,
        project_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<WikiSearchResult>, String> {
        Ok(self
            .search_pages_page(project_id, query, 0, top_k)
            .await?
            .results)
    }

    /// Search the materialized Wiki projection with SQLite FTS5/BM25.
    ///
    /// The projection keeps page bodies out of the hot query path: page files
    /// are read once when a page is written, while search only joins indexed
    /// rows and the small metadata table for status/page type.
    pub async fn search_pages_page(
        &self,
        project_id: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<WikiSearchPage, String> {
        let query = query.trim();
        let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
        let offset = offset.min(usize::MAX.saturating_sub(limit));
        let Some(match_query) = fts_query(query) else {
            return Ok(WikiSearchPage {
                results: Vec::new(),
                total: 0,
                offset,
                limit,
                query: query.to_string(),
            });
        };

        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM wiki_page_search
             JOIN wiki_pages ON wiki_pages.id = wiki_page_search.page_id
             WHERE wiki_page_search.project_id = ?
               AND wiki_pages.status = 'active'
               AND wiki_page_search MATCH ?",
        )
        .bind(project_id)
        .bind(&match_query)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let rows = sqlx::query_as::<_, (String, String, String, String, f64, String)>(
            "SELECT wiki_page_search.page_id,
                    wiki_page_search.path,
                    wiki_page_search.title,
                    wiki_pages.page_type,
                    bm25(wiki_page_search, 0.0, 0.0, 6.0, 8.0, 2.0, 4.0) AS rank,
                    snippet(wiki_page_search, 4, '', '', '...', 24) AS snippet
             FROM wiki_page_search
             JOIN wiki_pages ON wiki_pages.id = wiki_page_search.page_id
             WHERE wiki_page_search.project_id = ?
               AND wiki_pages.status = 'active'
               AND wiki_page_search MATCH ?
             ORDER BY rank ASC, wiki_page_search.path ASC
             LIMIT ? OFFSET ?",
        )
        .bind(project_id)
        .bind(&match_query)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let results = rows
            .into_iter()
            .map(|(page_id, path, title, page_type, rank, snippet)| WikiSearchResult {
                page_id,
                path,
                title,
                page_type,
                // FTS5 BM25 ranks lower (negative) values first. The logistic
                // transform preserves that ordering while keeping the public
                // display score in the familiar 0..1 interval.
                score: 1.0 / (1.0 + rank.exp()),
                snippet,
            })
            .collect();

        Ok(WikiSearchPage {
            results,
            total,
            offset,
            limit,
            query: query.to_string(),
        })
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
        let spec = TaskSpec::new("wiki", task_type, "wiki_project", project_id)
            .subject_id(Some(source_id.to_string()))
            .idempotency_key(format!("wiki:{}:{}:{}", task_type, project_id, source_id))
            .payload(serde_json::json!({
                "payload_version": 1,
                "project_id": project_id,
                "source_id": source_id,
            }))
            .auto_resume(true)
            .total_items(3);
        TaskRepository::new(self.pool.clone())
            .create_if_idle(&spec)
            .await
            .map(|task| task.map(|task| task.id))
            .map_err(|e| format!("DB error: {}", e))
    }

    pub async fn update_task_status(&self, task_id: &str, status: &str, progress: i64, done_steps: i64, total_steps: i64, result: Option<&str>, error: Option<&str>) -> Result<(), String> {
        let tasks = TaskRepository::new(self.pool.clone());
        let operation = async {
            match status {
                "running" => {
                    let task = tasks.get(task_id).await?;
                    if task.status == "pending" {
                        tasks.claim(task_id, wiki_stage(progress)).await?;
                    }
                    tasks
                        .update_progress(
                            task_id,
                            wiki_stage(progress),
                            progress,
                            done_steps,
                            total_steps,
                        )
                        .await
                }
                "done" | "completed" | "succeeded" => {
                    let task = tasks.get(task_id).await?;
                    if task.status == "pending" {
                        tasks.claim(task_id, "completed").await?;
                    }
                    tasks.succeed(task_id, result).await
                }
                "failed" => tasks.fail(task_id, error.unwrap_or("Wiki 来源摄入失败")).await,
                "cancelled" => tasks.mark_cancelled(task_id).await,
                _ => Err(sqlx::Error::Protocol(format!("unsupported task status: {}", status))),
            }
        }
        .await;
        operation.map_err(|e| format!("DB error: {}", e))
    }

    pub async fn list_tasks(&self, project_id: &str) -> Result<Vec<WikiIngestTask>, String> {
        TaskRepository::new(self.pool.clone())
            .list(&TaskListFilter {
                domain: Some("wiki".to_string()),
                resource_type: Some("wiki_project".to_string()),
                resource_id: Some(project_id.to_string()),
                limit: Some(20),
                ..TaskListFilter::default()
            })
            .await
            .map(|tasks| tasks.into_iter().map(background_to_wiki_task).collect())
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

/// Reconcile the FTS projection after an upgrade.  Older CrowAPI versions
/// stored page metadata and bodies separately, so a migration cannot populate
/// the virtual table by itself.  The count/missing-row check keeps normal
/// startups cheap while still repairing partially populated indexes.
pub async fn rebuild_search_index(pool: &SqlitePool) -> Result<(), String> {
    let missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM wiki_pages
         LEFT JOIN wiki_page_search ON wiki_page_search.page_id = wiki_pages.id
         WHERE wiki_pages.status = 'active' AND wiki_page_search.page_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;
    let orphaned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM wiki_page_search
         LEFT JOIN wiki_pages ON wiki_pages.id = wiki_page_search.page_id
         WHERE wiki_pages.id IS NULL OR wiki_pages.status <> 'active'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;
    if missing == 0 && orphaned == 0 {
        return Ok(());
    }

    let pages = sqlx::query_as::<_, WikiPage>(
        "SELECT * FROM wiki_pages WHERE status = 'active' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let mut documents = Vec::with_capacity(pages.len());
    for page in pages {
        let content = match crate::services::wiki::project::read_page(&page.project_id, &page.path).await {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(
                    %error,
                    project_id = %page.project_id,
                    page_path = %page.path,
                    "Wiki page body unavailable while rebuilding search index"
                );
                String::new()
            }
        };
        documents.push((page, content));
    }

    let mut tx = pool.begin().await.map_err(|e| format!("DB error: {}", e))?;
    sqlx::query("DELETE FROM wiki_page_search")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    for (page, content) in documents {
        sqlx::query(
            "INSERT INTO wiki_page_search (page_id, project_id, path, title, content, tags)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(page.id)
        .bind(page.project_id)
        .bind(page.path)
        .bind(page.title)
        .bind(content)
        .bind(page.tags)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    }
    tx.commit().await.map_err(|e| format!("DB error: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{fts_query, WikiRepository};
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn fts_query_quotes_operators_and_preserves_terms() {
        assert_eq!(fts_query("router OR secret"), Some("\"router\" AND \"OR\" AND \"secret\"".to_string()));
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("a\"b"), Some("\"a\"\"b\"".to_string()));
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

    #[tokio::test]
    async fn fts_search_returns_bm25_results_with_pagination() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create Wiki FTS test database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        let now = "2026-08-20T00:00:00Z";
        sqlx::query(
            "INSERT INTO wiki_projects
             (id, name, status, wiki_dir, mcp_enabled, source_count, page_count, created_at, updated_at)
             VALUES ('fts-project', 'FTS', 1, '/tmp/wiki', 1, 0, 0, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert project");

        let repo = WikiRepository::new(pool.clone());
        repo.upsert_page(
            "fts-project",
            "concepts/router.md",
            "Router",
            "concept",
            "hash-1",
            3,
            "[]",
            "{}",
            "[\"http\"]",
            "The router dispatches HTTP requests to handlers.",
        )
        .await
        .expect("index first page");
        repo.upsert_page(
            "fts-project",
            "entities/proxy.md",
            "Proxy",
            "entity",
            "hash-2",
            3,
            "[]",
            "{}",
            "[\"http\"]",
            "The proxy forwards requests through the router.",
        )
        .await
        .expect("index second page");

        let first = repo
            .search_pages_page("fts-project", "router", 0, 1)
            .await
            .expect("search pages");
        assert_eq!(first.total, 2);
        assert_eq!(first.results.len(), 1);
        assert!(first.results[0].snippet.contains("router"));
        assert_eq!(first.offset, 0);
        assert_eq!(first.limit, 1);

        let second = repo
            .search_pages_page("fts-project", "router", 1, 1)
            .await
            .expect("search second page");
        assert_eq!(second.total, 2);
        assert_eq!(second.results.len(), 1);
        assert_ne!(first.results[0].page_id, second.results[0].page_id);

        repo.delete_page("fts-project", "concepts/router.md")
            .await
            .expect("delete indexed page");
        let after_delete = repo
            .search_pages_page("fts-project", "router", 0, 10)
            .await
            .expect("search after delete");
        assert_eq!(after_delete.total, 1);
    }
}

fn wiki_stage(progress: i64) -> &'static str {
    match progress {
        0..=9 => "preparing",
        10..=29 => "parsing",
        30..=79 => "generating",
        80..=99 => "linking",
        _ => "completed",
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
