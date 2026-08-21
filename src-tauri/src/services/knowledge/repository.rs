use super::models::*;
use sqlx::{Sqlite, SqlitePool, Transaction};
use crate::db::models::now_iso;
use crate::services::tasks::{
    models::{BackgroundTask, TaskListFilter, TaskSpec},
    repository::TaskRepository,
};

fn background_to_kb_task(task: BackgroundTask) -> KbTask {
    KbTask {
        id: task.id,
        kb_id: task.resource_id,
        doc_id: task.subject_id,
        task_type: task.task_type,
        status: if task.status == "succeeded" {
            "completed".to_string()
        } else {
            task.status
        },
        progress: task.progress,
        total_items: task.total_items,
        done_items: task.done_items,
        error_message: task.error_message,
        created_at: task.created_at,
        completed_at: task.completed_at,
    }
}

pub struct KbRepository {
    pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct IndexSnapshot {
    pub content_revision: i64,
    pub config_revision: i64,
    pub chunks: Vec<(String, String, String, Vec<u8>, String, String)>,
}

#[derive(Debug, Clone)]
pub struct KbUpdateOutcome {
    pub knowledge_base: KbKnowledgeBase,
    pub reprocess_required: bool,
    pub reprocess_task_id: Option<String>,
}

pub const KB_CONFIG_SUPERSEDED: &str = "KB_CONFIG_SUPERSEDED";

impl KbRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn refresh_content_state(
        tx: &mut Transaction<'_, Sqlite>,
        kb_id: &str,
        now: &str,
    ) -> Result<(), sqlx::Error> {
        let has_chunks: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM kb_chunks c
                JOIN kb_documents d ON d.id = c.doc_id
                WHERE c.kb_id = ?
                  AND c.embedding IS NOT NULL
                  AND d.status = 'ready'
                  AND d.processed_config_revision = (
                      SELECT config_revision FROM kb_knowledge_bases WHERE id = ?
                  )
            )",
        )
        .bind(kb_id)
        .bind(kb_id)
        .fetch_one(&mut **tx)
        .await?;
        let index_status = if has_chunks { "stale" } else { "none" };

        let updated = sqlx::query(
            "UPDATE kb_knowledge_bases
             SET doc_count = (SELECT COUNT(*) FROM kb_documents WHERE kb_id = ?),
                 chunk_count = (SELECT COUNT(*) FROM kb_chunks WHERE kb_id = ?),
                 total_tokens = (SELECT COALESCE(SUM(token_count), 0) FROM kb_chunks WHERE kb_id = ?),
                 content_revision = content_revision + 1,
                 index_status = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(kb_id)
        .bind(kb_id)
        .bind(kb_id)
        .bind(index_status)
        .bind(now)
        .bind(kb_id)
        .execute(&mut **tx)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }

        let content_revision: i64 = sqlx::query_scalar(
            "SELECT content_revision FROM kb_knowledge_bases WHERE id = ?",
        )
        .bind(kb_id)
        .fetch_one(&mut **tx)
        .await?;

        if has_chunks {
            sqlx::query("UPDATE kb_index_meta SET status = 'stale' WHERE kb_id = ?")
                .bind(kb_id)
                .execute(&mut **tx)
                .await?;
        } else {
            sqlx::query(
                "UPDATE kb_index_meta
                 SET status = 'none', indexed_revision = ?, index_path = NULL, built_at = NULL,
                     format_version = 0, config_revision = 0,
                     content_fingerprint = NULL, index_checksum = NULL
                 WHERE kb_id = ?",
            )
            .bind(content_revision)
            .bind(kb_id)
            .execute(&mut **tx)
            .await?;
        }

        Ok(())
    }

    // ==================== Knowledge Base ====================

    pub async fn get_all_kbs(&self) -> Result<Vec<KbKnowledgeBase>, sqlx::Error> {
        sqlx::query_as::<_, KbKnowledgeBase>(
            "SELECT * FROM kb_knowledge_bases ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_kb(&self, id: &str) -> Result<KbKnowledgeBase, sqlx::Error> {
        sqlx::query_as::<_, KbKnowledgeBase>("SELECT * FROM kb_knowledge_bases WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn create_kb(&self, input: &CreateKbInput) -> Result<KbKnowledgeBase, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        sqlx::query(
            "INSERT INTO kb_knowledge_bases (id, name, description, status, doc_count, chunk_count, total_tokens, embedding_model, embedding_channel_id, mcp_enabled, chunk_size, chunk_overlap, excluded_dirs, excluded_files, included_files, embedding_dim, index_status, embedding_batch_size, created_at, updated_at)
             VALUES (?, ?, ?, 1, 0, 0, 0, ?, ?, 1, 512, 64, '', '', '', 0, 'none', 32, ?, ?)"
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.embedding_model)
        .bind(&input.embedding_channel_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_kb(&id).await
    }

    pub async fn update_kb_with_effects(
        &self,
        id: &str,
        input: &UpdateKbInput,
    ) -> Result<KbUpdateOutcome, sqlx::Error> {
        let now = now_iso();
        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_as::<_, KbKnowledgeBase>(
            "SELECT * FROM kb_knowledge_bases WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

        let requested_embedding_channel = input.embedding_channel_id.as_ref().map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        });
        let embedding_identity_changed = input.embedding_model.as_ref().is_some_and(|value| {
            current.embedding_model.as_ref() != Some(value)
        }) || requested_embedding_channel.is_some_and(|value| {
            current.embedding_channel_id.as_deref() != value
        });
        let processing_config_changed = embedding_identity_changed
            || input.chunk_size.is_some_and(|value| value != current.chunk_size)
            || input.chunk_overlap.is_some_and(|value| value != current.chunk_overlap);
        let has_documents: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM kb_documents WHERE kb_id = ?)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        let has_chunks: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM kb_chunks WHERE kb_id = ?)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

        let mut q = sqlx::QueryBuilder::new("UPDATE kb_knowledge_bases SET updated_at = ");
        q.push_bind(&now);

        if let Some(name) = &input.name {
            q.push(", name = ").push_bind(name);
        }
        if let Some(desc) = &input.description {
            q.push(", description = ").push_bind(desc);
        }
        if let Some(model) = &input.embedding_model {
            q.push(", embedding_model = ").push_bind(model);
        }
        if let Some(ch) = &input.embedding_channel_id {
            if ch.trim().is_empty() {
                q.push(", embedding_channel_id = NULL");
            } else {
                q.push(", embedding_channel_id = ").push_bind(ch.trim());
            }
        }
        if let Some(status) = input.status {
            q.push(", status = ").push_bind(status);
        }
        if let Some(mcp_enabled) = input.mcp_enabled {
            q.push(", mcp_enabled = ").push_bind(mcp_enabled);
        }
        if let Some(chunk_size) = input.chunk_size {
            q.push(", chunk_size = ").push_bind(chunk_size);
        }
        if let Some(chunk_overlap) = input.chunk_overlap {
            q.push(", chunk_overlap = ").push_bind(chunk_overlap);
        }
        if let Some(excluded_dirs) = &input.excluded_dirs {
            q.push(", excluded_dirs = ").push_bind(excluded_dirs);
        }
        if let Some(excluded_files) = &input.excluded_files {
            q.push(", excluded_files = ").push_bind(excluded_files);
        }
        if let Some(included_files) = &input.included_files {
            q.push(", included_files = ").push_bind(included_files);
        }
        if let Some(embedding_batch_size) = input.embedding_batch_size {
            q.push(", embedding_batch_size = ").push_bind(embedding_batch_size.max(1));
        }

        if processing_config_changed {
            q.push(", config_revision = config_revision + 1")
                .push(", content_revision = content_revision + 1")
                .push(", index_status = ")
                .push_bind(if has_chunks { "stale" } else { "none" });
            if embedding_identity_changed {
                q.push(", embedding_dim = 0");
            }
        }

        q.push(" WHERE id = ").push_bind(id);
        let updated = q.build().execute(&mut *tx).await?;
        if updated.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }

        if processing_config_changed {
            sqlx::query(
                "UPDATE kb_documents
                 SET status = 'stale', error_message = NULL, updated_at = ?
                 WHERE kb_id = ? AND status IN ('pending', 'processing', 'ready', 'cancelled')",
            )
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            if has_chunks {
                sqlx::query("UPDATE kb_index_meta SET status = 'stale' WHERE kb_id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query(
                    "UPDATE kb_index_meta
                     SET status = 'none', index_path = NULL, built_at = NULL,
                         format_version = 0, config_revision = 0,
                         content_fingerprint = NULL, index_checksum = NULL
                     WHERE kb_id = ?",
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "UPDATE background_tasks
                 SET cancel_requested = 1,
                     status = CASE
                         WHEN status = 'pending' OR task_type = 'reprocess_knowledge_base'
                             THEN 'cancelled'
                         ELSE status
                     END,
                     stage = CASE
                         WHEN status = 'pending' OR task_type = 'reprocess_knowledge_base'
                             THEN 'cancelled'
                         ELSE stage
                     END,
                     completed_at = CASE
                         WHEN status = 'pending' OR task_type = 'reprocess_knowledge_base'
                             THEN ?
                         ELSE completed_at
                     END,
                     updated_at = ?
                 WHERE domain = 'knowledge'
                   AND resource_type = 'knowledge_base'
                   AND resource_id = ?
                   AND task_type IN (
                       'process_document',
                       'reindex_document',
                       'build_index',
                       'reprocess_knowledge_base'
                   )
                   AND status IN ('pending', 'running')",
            )
            .bind(&now)
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }

        let knowledge_base = sqlx::query_as::<_, KbKnowledgeBase>(
            "SELECT * FROM kb_knowledge_bases WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        let reprocess_task_id = if processing_config_changed && has_documents {
            let task_id = uuid::Uuid::new_v4().to_string();
            let idempotency_key = format!("knowledge:reprocess:{}", id);
            let total_items: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)
                 FROM kb_documents
                 WHERE kb_id = ? AND processed_config_revision != ?",
            )
            .bind(id)
            .bind(knowledge_base.config_revision)
            .fetch_one(&mut *tx)
            .await?;
            let payload_json = serde_json::json!({
                "payload_version": 1,
                "operation": "reprocess_knowledge_base",
                "kb_id": id,
                "config_revision": knowledge_base.config_revision,
            })
            .to_string();
            sqlx::query(
                "INSERT INTO background_tasks
                 (id, domain, task_type, resource_type, resource_id, subject_id,
                  parent_task_id, idempotency_key, status, stage, progress, total_items,
                  done_items, payload_json, retryable, auto_resume, cancel_requested, attempt,
                  created_at, updated_at)
                 VALUES (?, 'knowledge', 'reprocess_knowledge_base', 'knowledge_base', ?, NULL,
                         NULL, ?, 'pending', 'queued', 0, ?, 0, ?, 1, 1, 0, 1, ?, ?)",
            )
            .bind(&task_id)
            .bind(id)
            .bind(idempotency_key)
            .bind(total_items)
            .bind(payload_json)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            Some(task_id)
        } else {
            None
        };
        tx.commit().await?;
        Ok(KbUpdateOutcome {
            knowledge_base,
            reprocess_required: processing_config_changed && has_documents,
            reprocess_task_id,
        })
    }

    pub async fn update_kb_counts(&self, kb_id: &str) -> Result<(), sqlx::Error> {
        let now = now_iso();
        sqlx::query(
            "UPDATE kb_knowledge_bases
             SET doc_count = (SELECT COUNT(*) FROM kb_documents WHERE kb_id = ?),
                 chunk_count = (SELECT COUNT(*) FROM kb_chunks WHERE kb_id = ?),
                 total_tokens = (SELECT COALESCE(SUM(token_count), 0) FROM kb_chunks WHERE kb_id = ?),
                 updated_at = ?
             WHERE id = ?"
        )
            .bind(kb_id)
            .bind(kb_id)
            .bind(kb_id)
            .bind(&now)
            .bind(kb_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn update_kb_index_status(&self, kb_id: &str, status: &str) -> Result<(), sqlx::Error> {
        let now = now_iso();
        sqlx::query("UPDATE kb_knowledge_bases SET index_status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(&now)
            .bind(kb_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_kb_embedding_dim(&self, kb_id: &str, dim: i64) -> Result<(), sqlx::Error> {
        let now = now_iso();
        sqlx::query("UPDATE kb_knowledge_bases SET embedding_dim = ?, updated_at = ? WHERE id = ?")
            .bind(dim)
            .bind(&now)
            .bind(kb_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ==================== Document ====================

    pub async fn get_documents(&self, kb_id: &str) -> Result<Vec<KbDocument>, sqlx::Error> {
        sqlx::query_as::<_, KbDocument>("SELECT * FROM kb_documents WHERE kb_id = ? ORDER BY created_at DESC")
            .bind(kb_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_documents_needing_config_revision(
        &self,
        kb_id: &str,
        config_revision: i64,
    ) -> Result<Vec<KbDocument>, sqlx::Error> {
        sqlx::query_as::<_, KbDocument>(
            "SELECT *
             FROM kb_documents
             WHERE kb_id = ? AND processed_config_revision != ?
             ORDER BY created_at, id",
        )
        .bind(kb_id)
        .bind(config_revision)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_document(&self, id: &str) -> Result<KbDocument, sqlx::Error> {
        sqlx::query_as::<_, KbDocument>("SELECT * FROM kb_documents WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_document_in_kb(
        &self,
        kb_id: &str,
        id: &str,
    ) -> Result<KbDocument, sqlx::Error> {
        sqlx::query_as::<_, KbDocument>(
            "SELECT * FROM kb_documents WHERE kb_id = ? AND id = ?",
        )
        .bind(kb_id)
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn find_document_by_hash(&self, kb_id: &str, hash: &str) -> Result<Option<KbDocument>, sqlx::Error> {
        sqlx::query_as::<_, KbDocument>("SELECT * FROM kb_documents WHERE kb_id = ? AND content_hash = ?")
            .bind(kb_id)
            .bind(hash)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn create_document(&self, kb_id: &str, filename: &str, file_path: Option<&str>, file_type: &str, file_size: i64, content_hash: &str) -> Result<KbDocument, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        sqlx::query(
            "INSERT INTO kb_documents (id, kb_id, filename, file_path, file_type, file_size, content_hash, chunk_count, token_count, status, source_type, doc_meta, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, 'pending', 'upload', '{}', ?, ?)"
        )
        .bind(&id)
        .bind(kb_id)
        .bind(filename)
        .bind(file_path)
        .bind(file_type)
        .bind(file_size)
        .bind(content_hash)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_document(&id).await
    }

    pub async fn create_document_with_source(
        &self,
        kb_id: &str,
        filename: &str,
        file_path: Option<&str>,
        file_type: &str,
        file_size: i64,
        content_hash: &str,
        source_id: Option<&str>,
        source_type: &str,
        source_url: Option<&str>,
        source_path: Option<&str>,
    ) -> Result<KbDocument, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        let result = if let Some(source_id) = source_id {
            sqlx::query(
                "INSERT INTO kb_documents (id, kb_id, filename, file_path, file_type, file_size, content_hash, chunk_count, token_count, status, source_id, source_type, source_url, source_path, doc_meta, created_at, updated_at)
                 SELECT ?, ?, ?, ?, ?, ?, ?, 0, 0, 'pending', ?, ?, ?, ?, '{}', ?, ?
                 FROM kb_sources
                 WHERE id = ? AND kb_id = ?"
            )
            .bind(&id)
            .bind(kb_id)
            .bind(filename)
            .bind(file_path)
            .bind(file_type)
            .bind(file_size)
            .bind(content_hash)
            .bind(source_id)
            .bind(source_type)
            .bind(source_url)
            .bind(source_path)
            .bind(&now)
            .bind(&now)
            .bind(source_id)
            .bind(kb_id)
            .execute(&self.pool)
            .await?
        } else {
            sqlx::query(
                "INSERT INTO kb_documents (id, kb_id, filename, file_path, file_type, file_size, content_hash, chunk_count, token_count, status, source_id, source_type, source_url, source_path, doc_meta, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, 'pending', NULL, ?, ?, ?, '{}', ?, ?)"
            )
            .bind(&id)
            .bind(kb_id)
            .bind(filename)
            .bind(file_path)
            .bind(file_type)
            .bind(file_size)
            .bind(content_hash)
            .bind(source_type)
            .bind(source_url)
            .bind(source_path)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await?
        };
        if result.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }

        self.get_document(&id).await
    }

    pub async fn update_document_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), sqlx::Error> {
        let now = now_iso();
        sqlx::query("UPDATE kb_documents SET status = ?, error_message = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(error)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_document_counts(&self, id: &str, chunk_count: i64, token_count: i64) -> Result<(), sqlx::Error> {
        let now = now_iso();
        sqlx::query("UPDATE kb_documents SET chunk_count = ?, token_count = ?, updated_at = ? WHERE id = ?")
            .bind(chunk_count)
            .bind(token_count)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_document_snapshot_path(
        &self,
        kb_id: &str,
        source_id: &str,
        doc_id: &str,
        file_path: &str,
    ) -> Result<(), sqlx::Error> {
        let updated = sqlx::query(
            "UPDATE kb_documents
             SET file_path = ?, updated_at = ?
             WHERE id = ? AND kb_id = ? AND source_id = ?",
        )
        .bind(file_path)
        .bind(now_iso())
        .bind(doc_id)
        .bind(kb_id)
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            Ok(())
        } else {
            Err(sqlx::Error::RowNotFound)
        }
    }

    pub async fn delete_document(&self, id: &str) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let mut tx = self.pool.begin().await?;
        let kb_id: String = sqlx::query_scalar("SELECT kb_id FROM kb_documents WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM kb_documents WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        Self::refresh_content_state(&mut tx, &kb_id, &now).await?;
        tx.commit().await
    }

    // ==================== Chunk ====================

    pub async fn create_chunk(&self, chunk: &ChunkInsert) -> Result<(), sqlx::Error> {
        // 从 metadata JSON 中提取 symbol_name / symbol_kind
        let meta: serde_json::Value = serde_json::from_str(&chunk.metadata).unwrap_or_default();
        let symbol_name = meta.get("symbol_name").and_then(|v| v.as_str());
        let symbol_kind = meta.get("symbol_kind").and_then(|v| v.as_str());

        let result = sqlx::query(
            "INSERT INTO kb_chunks (id, doc_id, kb_id, chunk_index, content, token_count, embedding, embedding_dim, metadata, symbol_name, symbol_kind, created_at)
             SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
             FROM kb_documents
             WHERE id = ? AND kb_id = ?"
        )
        .bind(&chunk.id)
        .bind(&chunk.doc_id)
        .bind(&chunk.kb_id)
        .bind(chunk.chunk_index)
        .bind(&chunk.content)
        .bind(chunk.token_count)
        .bind(&chunk.embedding)
        .bind(chunk.embedding_dim)
        .bind(&chunk.metadata)
        .bind(symbol_name)
        .bind(symbol_kind)
        .bind(&chunk.created_at)
        .bind(&chunk.doc_id)
        .bind(&chunk.kb_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        Ok(())
    }

    pub async fn delete_chunks_by_doc(&self, doc_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM kb_chunks WHERE doc_id = ?")
            .bind(doc_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn replace_document_chunks(
        &self,
        kb_id: &str,
        doc_id: &str,
        chunks: &[ChunkInsert],
        total_tokens: i64,
    ) -> Result<(), sqlx::Error> {
        let config_revision: i64 = sqlx::query_scalar(
            "SELECT config_revision FROM kb_knowledge_bases WHERE id = ?",
        )
        .bind(kb_id)
        .fetch_one(&self.pool)
        .await?;
        self.replace_document_chunks_for_config(
            kb_id,
            doc_id,
            chunks,
            total_tokens,
            config_revision,
        )
        .await
    }

    pub async fn replace_document_chunks_for_config(
        &self,
        kb_id: &str,
        doc_id: &str,
        chunks: &[ChunkInsert],
        total_tokens: i64,
        expected_config_revision: i64,
    ) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM kb_chunks WHERE doc_id = ?")
            .bind(doc_id)
            .execute(&mut *tx)
            .await?;

        for chunk in chunks {
            let metadata: serde_json::Value =
                serde_json::from_str(&chunk.metadata).unwrap_or_default();
            let symbol_name = metadata.get("symbol_name").and_then(|value| value.as_str());
            let symbol_kind = metadata.get("symbol_kind").and_then(|value| value.as_str());
            sqlx::query(
                "INSERT INTO kb_chunks
                 (id, doc_id, kb_id, chunk_index, content, token_count, embedding, embedding_dim, metadata, symbol_name, symbol_kind, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&chunk.id)
            .bind(doc_id)
            .bind(kb_id)
            .bind(chunk.chunk_index)
            .bind(&chunk.content)
            .bind(chunk.token_count)
            .bind(&chunk.embedding)
            .bind(chunk.embedding_dim)
            .bind(&chunk.metadata)
            .bind(symbol_name)
            .bind(symbol_kind)
            .bind(&chunk.created_at)
            .execute(&mut *tx)
            .await?;
        }

        let updated = sqlx::query(
            "UPDATE kb_documents
             SET chunk_count = ?, token_count = ?, status = 'ready', error_message = NULL,
                 processed_config_revision = ?, updated_at = ?
             WHERE id = ? AND kb_id = ?
               AND EXISTS (
                   SELECT 1 FROM kb_knowledge_bases
                   WHERE id = ? AND config_revision = ?
               )",
        )
        .bind(chunks.len() as i64)
        .bind(total_tokens)
        .bind(expected_config_revision)
        .bind(&now)
        .bind(doc_id)
        .bind(kb_id)
        .bind(kb_id)
        .bind(expected_config_revision)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(sqlx::Error::Protocol(KB_CONFIG_SUPERSEDED.to_string()));
        }

        Self::refresh_content_state(&mut tx, kb_id, &now).await?;

        tx.commit().await
    }

    pub async fn get_index_snapshot(&self, kb_id: &str) -> Result<IndexSnapshot, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let (content_revision, config_revision): (i64, i64) = sqlx::query_as(
            "SELECT content_revision, config_revision FROM kb_knowledge_bases WHERE id = ?",
        )
        .bind(kb_id)
        .fetch_one(&mut *tx)
        .await?;
        let chunks = sqlx::query_as(
            "SELECT c.id, c.content, c.metadata, c.embedding, d.filename, c.doc_id
             FROM kb_chunks c
             JOIN kb_documents d ON c.doc_id = d.id
             JOIN kb_knowledge_bases kb ON kb.id = c.kb_id
             WHERE c.kb_id = ? AND c.embedding IS NOT NULL AND d.status = 'ready'
               AND d.processed_config_revision = kb.config_revision
             ORDER BY d.id, c.chunk_index, c.id",
        )
        .bind(kb_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(IndexSnapshot {
            content_revision,
            config_revision,
            chunks,
        })
    }

    pub async fn get_chunks_by_kb(&self, kb_id: &str) -> Result<Vec<(String, String, String, Vec<u8>, String, String)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT c.id, c.content, c.metadata, c.embedding, d.filename, c.doc_id
             FROM kb_chunks c
             JOIN kb_documents d ON c.doc_id = d.id
             JOIN kb_knowledge_bases kb ON kb.id = c.kb_id
             WHERE c.kb_id = ? AND c.embedding IS NOT NULL AND d.status = 'ready'
               AND d.processed_config_revision = kb.config_revision
             ORDER BY d.id, c.chunk_index, c.id"
        )
        .bind(kb_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_chunks_by_kb_with_dim(&self, kb_id: &str) -> Result<Vec<ChunkWithEmbedding>, sqlx::Error> {
        sqlx::query_as(
            "SELECT c.id, c.content, c.metadata, c.embedding, c.embedding_dim, d.filename, c.doc_id
             FROM kb_chunks c
             JOIN kb_documents d ON c.doc_id = d.id
             JOIN kb_knowledge_bases kb ON kb.id = c.kb_id
             WHERE c.kb_id = ? AND c.embedding IS NOT NULL AND d.status = 'ready'
               AND d.processed_config_revision = kb.config_revision
             ORDER BY d.id, c.chunk_index, c.id"
        )
        .bind(kb_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_chunk_count_by_kb(&self, kb_id: &str) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM kb_chunks c
             JOIN kb_documents d ON d.id = c.doc_id
             JOIN kb_knowledge_bases kb ON kb.id = c.kb_id
             WHERE c.kb_id = ? AND c.embedding IS NOT NULL AND d.status = 'ready'
               AND d.processed_config_revision = kb.config_revision",
        )
            .bind(kb_id)
            .fetch_one(&self.pool)
            .await
    }

    // ==================== Task ====================

    pub async fn create_task(&self, kb_id: &str, doc_id: Option<&str>, task_type: &str, total_items: i64) -> Result<KbTask, sqlx::Error> {
        self.create_task_if_idle(kb_id, doc_id, task_type, total_items)
            .await?
            .ok_or_else(|| sqlx::Error::Protocol("knowledge task already running".to_string()))
    }

    pub async fn create_task_if_idle(
        &self,
        kb_id: &str,
        doc_id: Option<&str>,
        task_type: &str,
        total_items: i64,
    ) -> Result<Option<KbTask>, sqlx::Error> {
        self.create_task_if_idle_with_options(
            kb_id,
            doc_id,
            task_type,
            total_items,
            None,
            true,
        )
        .await
    }

    pub async fn create_task_if_idle_with_options(
        &self,
        kb_id: &str,
        doc_id: Option<&str>,
        task_type: &str,
        total_items: i64,
        parent_task_id: Option<&str>,
        retryable: bool,
    ) -> Result<Option<KbTask>, sqlx::Error> {
        let idempotency_key = match doc_id {
            Some(doc_id) => format!("knowledge:document:{}", doc_id),
            None => format!("knowledge:{}:{}", task_type, kb_id),
        };
        let spec = TaskSpec::new("knowledge", task_type, "knowledge_base", kb_id)
            .subject_id(doc_id.map(str::to_string))
            .parent_task_id(parent_task_id.map(str::to_string))
            .idempotency_key(idempotency_key)
            .payload(serde_json::json!({
                "payload_version": 1,
                "operation": task_type,
                "kb_id": kb_id,
                "doc_id": doc_id,
            }))
            .retryable(retryable)
            .auto_resume(retryable && parent_task_id.is_none())
            .total_items(total_items);
        let tasks = TaskRepository::new(self.pool.clone());
        let Some(task) = tasks.create_if_idle(&spec).await? else {
            return Ok(None);
        };
        if !tasks.claim(&task.id, "running").await? {
            return Ok(None);
        }
        tasks.get(&task.id).await.map(background_to_kb_task).map(Some)
    }

    pub async fn update_task_progress(&self, id: &str, done_items: i64, progress: i64) -> Result<(), sqlx::Error> {
        let tasks = TaskRepository::new(self.pool.clone());
        let task = tasks.get(id).await?;
        tasks
            .update_progress(id, &task.stage, progress, done_items, task.total_items)
            .await
    }

    pub async fn complete_task(&self, id: &str, error: Option<&str>) -> Result<(), sqlx::Error> {
        let tasks = TaskRepository::new(self.pool.clone());
        match error {
            Some(error) => tasks.fail(id, error).await,
            None => tasks.succeed(id, None).await,
        }
    }

    pub async fn get_tasks(&self, kb_id: &str) -> Result<Vec<KbTask>, sqlx::Error> {
        TaskRepository::new(self.pool.clone())
            .list(&TaskListFilter {
                domain: Some("knowledge".to_string()),
                resource_type: Some("knowledge_base".to_string()),
                resource_id: Some(kb_id.to_string()),
                limit: Some(20),
                ..TaskListFilter::default()
            })
            .await
            .map(|tasks| tasks.into_iter().map(background_to_kb_task).collect())
    }

    // ==================== Conversation History ====================

    pub async fn get_conversations(&self, kb_id: &str) -> Result<Vec<KbConversation>, sqlx::Error> {
        sqlx::query_as::<_, KbConversation>(
            "SELECT * FROM kb_conversations WHERE kb_id = ? ORDER BY created_at ASC"
        )
        .bind(kb_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn add_conversation(&self, kb_id: &str, role: &str, content: &str, sources: Option<&str>, model: Option<&str>, tokens_used: i64) -> Result<(), sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        sqlx::query(
            "INSERT INTO kb_conversations (id, kb_id, role, content, sources, model, tokens_used, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(kb_id)
        .bind(role)
        .bind(content)
        .bind(sources)
        .bind(model)
        .bind(tokens_used)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn clear_conversations(&self, kb_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM kb_conversations WHERE kb_id = ?")
            .bind(kb_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ==================== Sources ====================

    pub async fn get_sources(&self, kb_id: &str) -> Result<Vec<KbSource>, sqlx::Error> {
        sqlx::query_as::<_, KbSource>(
            "SELECT * FROM kb_sources WHERE kb_id = ? ORDER BY created_at DESC"
        )
        .bind(kb_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_source(&self, id: &str) -> Result<KbSource, sqlx::Error> {
        sqlx::query_as::<_, KbSource>("SELECT * FROM kb_sources WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn create_source(&self, kb_id: &str, source_type: &str, source_url: Option<&str>, source_path: Option<&str>, branch: Option<&str>) -> Result<KbSource, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        sqlx::query(
            "INSERT INTO kb_sources (id, kb_id, source_type, source_url, source_path, branch, status, file_count, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'fetching', 0, ?, ?)"
        )
        .bind(&id)
        .bind(kb_id)
        .bind(source_type)
        .bind(source_url)
        .bind(source_path)
        .bind(branch)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        sqlx::query_as::<_, KbSource>("SELECT * FROM kb_sources WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn update_source_status(&self, id: &str, status: &str, file_count: i64, error: Option<&str>) -> Result<(), sqlx::Error> {
        let now = now_iso();
        sqlx::query("UPDATE kb_sources SET status = ?, file_count = ?, error = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(file_count)
            .bind(error)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn count_documents_by_source(&self, source_id: &str) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM kb_documents WHERE source_id = ?")
            .bind(source_id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn delete_source(&self, id: &str) -> Result<(), sqlx::Error> {
        let (kb_id,): (String,) = sqlx::query_as("SELECT kb_id FROM kb_sources WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        self.delete_source_with_documents(&kb_id, id).await.map(|_| ())
    }

    /// Delete an import source and all documents created by that source in a
    /// single database transaction. Existing documents without source_id are
    /// intentionally left untouched because they cannot be mapped reliably.
    pub async fn delete_source_with_documents(
        &self,
        kb_id: &str,
        source_id: &str,
    ) -> Result<Vec<KbDocument>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let documents = sqlx::query_as::<_, KbDocument>(
            "SELECT * FROM kb_documents WHERE kb_id = ? AND source_id = ?"
        )
        .bind(kb_id)
        .bind(source_id)
        .fetch_all(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM kb_chunks WHERE doc_id IN (
                SELECT id FROM kb_documents WHERE kb_id = ? AND source_id = ?
            )"
        )
        .bind(kb_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM kb_documents WHERE kb_id = ? AND source_id = ?")
            .bind(kb_id)
            .bind(source_id)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM kb_sources WHERE id = ? AND kb_id = ?")
            .bind(source_id)
            .bind(kb_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        if !documents.is_empty() {
            let now = now_iso();
            Self::refresh_content_state(&mut tx, kb_id, &now).await?;
        }
        tx.commit().await?;
        Ok(documents)
    }

    // ==================== Index Meta ====================

    pub async fn get_index_meta(&self, kb_id: &str) -> Result<Option<KbIndexMeta>, sqlx::Error> {
        sqlx::query_as::<_, KbIndexMeta>("SELECT * FROM kb_index_meta WHERE kb_id = ?")
            .bind(kb_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn update_index_meta_status(&self, kb_id: &str, status: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE kb_index_meta SET status = ? WHERE kb_id = ?")
            .bind(status)
            .bind(kb_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_index_meta(
        &self,
        kb_id: &str,
        dim: i64,
        chunk_count: i64,
        index_path: Option<&str>,
        status: &str,
        indexed_revision: i64,
    ) -> Result<(), sqlx::Error> {
        self.upsert_index_meta_with_manifest(
            kb_id,
            dim,
            chunk_count,
            index_path,
            status,
            indexed_revision,
            0,
            0,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_index_meta_with_manifest(
        &self,
        kb_id: &str,
        dim: i64,
        chunk_count: i64,
        index_path: Option<&str>,
        status: &str,
        indexed_revision: i64,
        format_version: i64,
        config_revision: i64,
        content_fingerprint: Option<&str>,
        index_checksum: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let now = now_iso();
        sqlx::query(
            "INSERT INTO kb_index_meta
             (kb_id, index_type, embedding_dim, chunk_count, index_path, built_at, status,
              indexed_revision, format_version, config_revision, content_fingerprint, index_checksum)
             VALUES (?, 'hnsw', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(kb_id) DO UPDATE SET
                 embedding_dim = excluded.embedding_dim,
                 chunk_count = excluded.chunk_count,
                 index_path = excluded.index_path,
                 built_at = excluded.built_at,
                 status = excluded.status,
                 indexed_revision = excluded.indexed_revision,
                 format_version = excluded.format_version,
                 config_revision = excluded.config_revision,
                 content_fingerprint = excluded.content_fingerprint,
                 index_checksum = excluded.index_checksum"
        )
        .bind(kb_id)
        .bind(dim)
        .bind(chunk_count)
        .bind(index_path)
        .bind(&now)
        .bind(status)
        .bind(indexed_revision)
        .bind(format_version)
        .bind(config_revision)
        .bind(content_fingerprint)
        .bind(index_checksum)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn commit_index_snapshot(
        &self,
        kb_id: &str,
        expected_content_revision: i64,
        expected_config_revision: i64,
        dim: i64,
        chunk_count: i64,
        index_path: &str,
        format_version: i64,
        content_fingerprint: &str,
        index_checksum: &str,
    ) -> Result<bool, sqlx::Error> {
        let now = now_iso();
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
             "UPDATE kb_knowledge_bases
             SET index_status = 'ready', updated_at = ?
             WHERE id = ? AND content_revision = ? AND config_revision = ?",
        )
        .bind(&now)
        .bind(kb_id)
        .bind(expected_content_revision)
        .bind(expected_config_revision)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }

        sqlx::query(
            "INSERT INTO kb_index_meta
             (kb_id, index_type, embedding_dim, chunk_count, index_path, built_at, status,
              indexed_revision, format_version, config_revision, content_fingerprint, index_checksum)
             VALUES (?, 'hnsw', ?, ?, ?, ?, 'ready', ?, ?, ?, ?, ?)
             ON CONFLICT(kb_id) DO UPDATE SET
                 embedding_dim = excluded.embedding_dim,
                 chunk_count = excluded.chunk_count,
                 index_path = excluded.index_path,
                 built_at = excluded.built_at,
                 status = excluded.status,
                 indexed_revision = excluded.indexed_revision,
                 format_version = excluded.format_version,
                 config_revision = excluded.config_revision,
                 content_fingerprint = excluded.content_fingerprint,
                 index_checksum = excluded.index_checksum",
        )
        .bind(kb_id)
        .bind(dim)
        .bind(chunk_count)
        .bind(index_path)
        .bind(&now)
        .bind(expected_content_revision)
        .bind(format_version)
        .bind(expected_config_revision)
        .bind(content_fingerprint)
        .bind(index_checksum)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn mark_index_dropped(&self, kb_id: &str) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let mut tx = self.pool.begin().await?;
        let (content_revision, config_revision): (i64, i64) = sqlx::query_as(
            "SELECT content_revision, config_revision FROM kb_knowledge_bases WHERE id = ?",
        )
        .bind(kb_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE kb_knowledge_bases SET index_status = 'none', updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(kb_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO kb_index_meta
             (kb_id, index_type, embedding_dim, chunk_count, index_path, built_at, status,
              indexed_revision, format_version, config_revision, content_fingerprint, index_checksum)
             VALUES (?, 'hnsw', 0, 0, NULL, NULL, 'none', ?, 0, ?, NULL, NULL)
             ON CONFLICT(kb_id) DO UPDATE SET
                 embedding_dim = 0,
                 chunk_count = 0,
                 index_path = NULL,
                 built_at = NULL,
                 status = 'none',
                 indexed_revision = excluded.indexed_revision,
                 format_version = 0,
                 config_revision = excluded.config_revision,
                 content_fingerprint = NULL,
                 index_checksum = NULL",
        )
        .bind(kb_id)
        .bind(content_revision)
        .bind(config_revision)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    pub async fn index_needs_rebuild(&self, kb_id: &str) -> Result<bool, sqlx::Error> {
        let kb = self.get_kb(kb_id).await?;
        let chunk_count = self.get_chunk_count_by_kb(kb_id).await?;
        if chunk_count == 0 {
            return Ok(false);
        }
        let meta = self.get_index_meta(kb_id).await?;
        Ok(match meta {
            Some(meta) => {
                kb.index_status != "ready"
                    || meta.status != "ready"
                    || meta.indexed_revision != kb.content_revision
                    || meta.format_version != KB_INDEX_FORMAT_VERSION
                    || meta.config_revision != kb.config_revision
                    || meta.content_fingerprint.is_none()
                    || meta.index_checksum.is_none()
                    || meta.chunk_count != chunk_count
            }
            None => true,
        })
    }
}

pub struct ChunkInsert {
    pub id: String,
    pub doc_id: String,
    pub kb_id: String,
    pub chunk_index: i64,
    pub content: String,
    pub token_count: i64,
    pub embedding: Vec<u8>,
    pub embedding_dim: i64,
    pub metadata: String,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChunkWithEmbedding {
    pub id: String,
    pub content: String,
    pub metadata: String,
    pub embedding: Vec<u8>,
    pub embedding_dim: i64,
    pub filename: String,
    pub doc_id: String,
}

#[cfg(test)]
mod tests {
    use super::{ChunkInsert, KbRepository};
    use crate::services::knowledge::models::{
        CreateKbInput, UpdateKbInput, KB_INDEX_FORMAT_VERSION,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create knowledge repository test database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        pool
    }

    fn chunk(id: &str, doc_id: &str, kb_id: &str, content: &str) -> ChunkInsert {
        ChunkInsert {
            id: id.to_string(),
            doc_id: doc_id.to_string(),
            kb_id: kb_id.to_string(),
            chunk_index: 0,
            content: content.to_string(),
            token_count: 2,
            embedding: vec![0, 0, 128, 63],
            embedding_dim: 1,
            metadata: "{}".to_string(),
            created_at: "2026-08-20T00:00:00Z".to_string(),
        }
    }

    fn update_input() -> UpdateKbInput {
        UpdateKbInput {
            name: None,
            description: None,
            embedding_model: None,
            embedding_channel_id: None,
            status: None,
            mcp_enabled: None,
            chunk_size: None,
            chunk_overlap: None,
            excluded_dirs: None,
            excluded_files: None,
            included_files: None,
            embedding_batch_size: None,
        }
    }

    #[tokio::test]
    async fn configuration_updates_create_one_recoverable_reprocess_task_per_revision() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool.clone());
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "config revision".to_string(),
                description: None,
                embedding_model: Some("embedding-a".to_string()),
                embedding_channel_id: Some("channel-a".to_string()),
            })
            .await
            .unwrap();
        let document = repo
            .create_document(&kb.id, "config.txt", None, "text", 4, "config-hash")
            .await
            .unwrap();
        repo.replace_document_chunks(
            &kb.id,
            &document.id,
            &[chunk("config-chunk", &document.id, &kb.id, "current")],
            2,
        )
        .await
        .unwrap();
        let active_document_task = repo
            .create_task_if_idle(&kb.id, Some(&document.id), "reindex_document", 1)
            .await
            .unwrap()
            .unwrap();
        let before = repo.get_kb(&kb.id).await.unwrap();

        let mut metadata = update_input();
        metadata.name = Some("renamed".to_string());
        let metadata_outcome = repo.update_kb_with_effects(&kb.id, &metadata).await.unwrap();
        assert!(!metadata_outcome.reprocess_required);
        assert!(metadata_outcome.reprocess_task_id.is_none());
        assert_eq!(metadata_outcome.knowledge_base.config_revision, before.config_revision);
        assert_eq!(metadata_outcome.knowledge_base.content_revision, before.content_revision);

        let mut changed = update_input();
        changed.chunk_size = Some(before.chunk_size + 128);
        let first = repo.update_kb_with_effects(&kb.id, &changed).await.unwrap();
        assert!(first.reprocess_required);
        let first_task_id = first.reprocess_task_id.clone().unwrap();
        assert_eq!(first.knowledge_base.config_revision, before.config_revision + 1);
        assert_eq!(first.knowledge_base.content_revision, before.content_revision + 1);
        assert_eq!(
            repo.get_document(&document.id).await.unwrap().status,
            "stale"
        );
        let first_task: (String, i64, String) = sqlx::query_as(
            "SELECT status, auto_resume, payload_json FROM background_tasks WHERE id = ?",
        )
        .bind(&first_task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(first_task.0, "pending");
        assert_eq!(first_task.1, 1);
        assert!(first_task.2.contains(&format!(
            "\"config_revision\":{}",
            first.knowledge_base.config_revision
        )));
        let cancelled: (i64, String) = sqlx::query_as(
            "SELECT cancel_requested, status FROM background_tasks WHERE id = ?",
        )
        .bind(&active_document_task.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cancelled, (1, "running".to_string()));

        let repeated = repo.update_kb_with_effects(&kb.id, &changed).await.unwrap();
        assert!(!repeated.reprocess_required);
        assert!(repeated.reprocess_task_id.is_none());
        assert_eq!(
            repeated.knowledge_base.config_revision,
            first.knowledge_base.config_revision
        );

        let mut changed_again = update_input();
        changed_again.chunk_overlap = Some(before.chunk_overlap + 16);
        let second = repo
            .update_kb_with_effects(&kb.id, &changed_again)
            .await
            .unwrap();
        assert!(second.reprocess_task_id.is_some());
        assert_ne!(second.reprocess_task_id.as_deref(), Some(first_task_id.as_str()));
        let previous_status: String = sqlx::query_scalar(
            "SELECT status FROM background_tasks WHERE id = ?",
        )
        .bind(first_task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(previous_status, "cancelled");
    }

    #[tokio::test]
    async fn empty_embedding_channel_clears_binding_and_reprocesses_documents() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool);
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "clear channel".to_string(),
                description: None,
                embedding_model: Some("embedding-a".to_string()),
                embedding_channel_id: Some("channel-a".to_string()),
            })
            .await
            .unwrap();
        repo.create_document(&kb.id, "channel.txt", None, "text", 4, "channel-hash")
            .await
            .unwrap();

        let mut input = update_input();
        input.embedding_channel_id = Some("  ".to_string());
        let outcome = repo.update_kb_with_effects(&kb.id, &input).await.unwrap();

        assert!(outcome.reprocess_required);
        assert!(outcome.reprocess_task_id.is_some());
        assert_eq!(outcome.knowledge_base.embedding_channel_id, None);
        assert_eq!(outcome.knowledge_base.config_revision, kb.config_revision + 1);
    }

    #[tokio::test]
    async fn configuration_change_and_task_cancellation_roll_back_together() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool.clone());
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "transaction".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .unwrap();
        let document = repo
            .create_document(&kb.id, "transaction.txt", None, "text", 4, "transaction-hash")
            .await
            .unwrap();
        repo.replace_document_chunks(
            &kb.id,
            &document.id,
            &[chunk("transaction-chunk", &document.id, &kb.id, "current")],
            2,
        )
        .await
        .unwrap();
        repo.create_task_if_idle(&kb.id, Some(&document.id), "reindex_document", 1)
            .await
            .unwrap()
            .unwrap();
        let before = repo.get_kb(&kb.id).await.unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_configuration_task_cancel
             BEFORE UPDATE ON background_tasks
             BEGIN
                 SELECT RAISE(ABORT, 'injected task cancellation failure');
             END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut changed = update_input();
        changed.chunk_size = Some(before.chunk_size + 64);
        assert!(repo.update_kb_with_effects(&kb.id, &changed).await.is_err());
        let after = repo.get_kb(&kb.id).await.unwrap();
        assert_eq!(after.config_revision, before.config_revision);
        assert_eq!(after.content_revision, before.content_revision);
        assert_eq!(repo.get_document(&document.id).await.unwrap().status, "ready");
    }

    #[tokio::test]
    async fn superseded_document_commit_rolls_back_and_is_excluded_from_index_snapshot() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool.clone());
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "superseded".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .unwrap();
        let document = repo
            .create_document(&kb.id, "superseded.txt", None, "text", 4, "superseded-hash")
            .await
            .unwrap();
        repo.replace_document_chunks_for_config(
            &kb.id,
            &document.id,
            &[chunk("old-config-chunk", &document.id, &kb.id, "old")],
            2,
            kb.config_revision,
        )
        .await
        .unwrap();
        let mut changed = update_input();
        changed.chunk_size = Some(kb.chunk_size + 64);
        repo.update_kb_with_effects(&kb.id, &changed).await.unwrap();

        let result = repo
            .replace_document_chunks_for_config(
                &kb.id,
                &document.id,
                &[chunk("late-chunk", &document.id, &kb.id, "late")],
                2,
                kb.config_revision,
            )
            .await;
        assert!(result.unwrap_err().to_string().contains("KB_CONFIG_SUPERSEDED"));
        let chunk_id: String = sqlx::query_scalar(
            "SELECT id FROM kb_chunks WHERE doc_id = ?",
        )
        .bind(&document.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(chunk_id, "old-config-chunk");
        assert!(repo.get_index_snapshot(&kb.id).await.unwrap().chunks.is_empty());
        assert!(repo.get_chunks_by_kb(&kb.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn chunk_replacement_is_atomic_and_marks_index_stale() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool.clone());
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "atomic test".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .expect("create knowledge base");
        let document = repo
            .create_document(&kb.id, "test.txt", None, "text", 4, "hash")
            .await
            .expect("create document");
        repo.replace_document_chunks(
            &kb.id,
            &document.id,
            &[chunk("old", &document.id, &kb.id, "old")],
            2,
        )
        .await
        .expect("store initial snapshot");
        let indexed_revision = repo
            .get_kb(&kb.id)
            .await
            .expect("read content revision")
            .content_revision;
        assert_eq!(indexed_revision, 1);
        repo.upsert_index_meta(
            &kb.id,
            1,
            1,
            Some("index"),
            "ready",
            indexed_revision,
        )
        .await
        .expect("create ready index metadata");
        repo.update_kb_index_status(&kb.id, "ready")
            .await
            .expect("mark index ready");

        let failed = repo
            .replace_document_chunks(
                &kb.id,
                &document.id,
                &[
                    chunk("duplicate", &document.id, &kb.id, "new"),
                    chunk("duplicate", &document.id, &kb.id, "newer"),
                ],
                4,
            )
            .await;
        assert!(failed.is_err());
        let content: String = sqlx::query_scalar(
            "SELECT content FROM kb_chunks WHERE doc_id = ?",
        )
        .bind(&document.id)
        .fetch_one(&pool)
        .await
        .expect("old snapshot remains after rollback");
        assert_eq!(content, "old");
        let after_failed_replacement = repo.get_kb(&kb.id).await.expect("read KB after rollback");
        assert_eq!(after_failed_replacement.content_revision, indexed_revision);
        assert_eq!(after_failed_replacement.index_status, "ready");

        repo.replace_document_chunks(
            &kb.id,
            &document.id,
            &[chunk("new", &document.id, &kb.id, "new")],
            2,
        )
        .await
        .expect("replace document snapshot");
        let content: String = sqlx::query_scalar(
            "SELECT content FROM kb_chunks WHERE doc_id = ?",
        )
        .bind(&document.id)
        .fetch_one(&pool)
        .await
        .expect("read replacement snapshot");
        assert_eq!(content, "new");
        let replaced_kb = repo.get_kb(&kb.id).await.expect("read KB");
        assert_eq!(replaced_kb.index_status, "stale");
        assert_eq!(replaced_kb.content_revision, indexed_revision + 1);
        let stale_meta = repo
            .get_index_meta(&kb.id)
            .await
            .expect("read index metadata")
            .expect("index metadata exists");
        assert_eq!(stale_meta.status, "stale");
        assert_eq!(stale_meta.indexed_revision, indexed_revision);

        assert!(!repo
            .commit_index_snapshot(
                &kb.id,
                indexed_revision,
                kb.config_revision,
                1,
                1,
                "stale-index",
                KB_INDEX_FORMAT_VERSION,
                "stale-fingerprint",
                "stale-checksum",
            )
            .await
            .expect("reject stale index snapshot"));
        assert!(repo
            .commit_index_snapshot(
                &kb.id,
                replaced_kb.content_revision,
                replaced_kb.config_revision,
                1,
                1,
                "current-index",
                KB_INDEX_FORMAT_VERSION,
                "current-fingerprint",
                "current-checksum",
            )
            .await
            .expect("commit current index snapshot"));
        let ready_meta = repo
            .get_index_meta(&kb.id)
            .await
            .expect("read committed index metadata")
            .expect("committed index metadata exists");
        assert_eq!(ready_meta.status, "ready");
        assert_eq!(ready_meta.indexed_revision, replaced_kb.content_revision);
    }

    #[tokio::test]
    async fn document_delete_rolls_back_with_counts_and_revision() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool.clone());
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "delete transaction".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .expect("create knowledge base");
        let document = repo
            .create_document(&kb.id, "delete.txt", None, "text", 4, "delete-hash")
            .await
            .expect("create document");
        repo.replace_document_chunks(
            &kb.id,
            &document.id,
            &[chunk("delete-chunk", &document.id, &kb.id, "content")],
            2,
        )
        .await
        .expect("store document snapshot");
        let revision = repo
            .get_kb(&kb.id)
            .await
            .expect("read initial revision")
            .content_revision;

        sqlx::query(
            "CREATE TRIGGER reject_kb_content_revision
             BEFORE UPDATE OF content_revision ON kb_knowledge_bases
             BEGIN
                 SELECT RAISE(ABORT, 'injected revision failure');
             END",
        )
        .execute(&pool)
        .await
        .expect("install failure trigger");
        assert!(repo.delete_document(&document.id).await.is_err());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kb_documents WHERE id = ?")
                .bind(&document.id)
                .fetch_one(&pool)
                .await
                .expect("count rolled back document"),
            1,
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kb_chunks WHERE doc_id = ?")
                .bind(&document.id)
                .fetch_one(&pool)
                .await
                .expect("count rolled back chunks"),
            1,
        );
        assert_eq!(
            repo.get_kb(&kb.id)
                .await
                .expect("read revision after rollback")
                .content_revision,
            revision,
        );

        sqlx::query("DROP TRIGGER reject_kb_content_revision")
            .execute(&pool)
            .await
            .expect("remove failure trigger");
        repo.delete_document(&document.id)
            .await
            .expect("delete document atomically");
        let deleted_kb = repo.get_kb(&kb.id).await.expect("read KB after delete");
        assert_eq!(deleted_kb.doc_count, 0);
        assert_eq!(deleted_kb.chunk_count, 0);
        assert_eq!(deleted_kb.total_tokens, 0);
        assert_eq!(deleted_kb.content_revision, revision + 1);
        assert_eq!(deleted_kb.index_status, "none");
    }

    #[tokio::test]
    async fn task_claim_allows_only_one_active_task() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool);
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "task test".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .expect("create knowledge base");
        let first = repo
            .create_task_if_idle(&kb.id, None, "build_index", 1)
            .await
            .expect("claim first task")
            .expect("first task is claimed");
        assert!(repo
            .create_task_if_idle(&kb.id, None, "build_index", 1)
            .await
            .expect("attempt duplicate claim")
            .is_none());
        repo.complete_task(&first.id, None)
            .await
            .expect("complete first task");
        assert!(repo
            .create_task_if_idle(&kb.id, None, "build_index", 1)
            .await
            .expect("claim replacement task")
            .is_some());
    }

    #[tokio::test]
    async fn document_lookup_is_scoped_to_its_knowledge_base() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool);
        let first_kb = repo
            .create_kb(&CreateKbInput {
                name: "first".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .expect("create first knowledge base");
        let second_kb = repo
            .create_kb(&CreateKbInput {
                name: "second".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .expect("create second knowledge base");
        let document = repo
            .create_document(&first_kb.id, "scope.txt", None, "text", 5, "scope-hash")
            .await
            .expect("create scoped document");

        assert_eq!(
            repo.get_document_in_kb(&first_kb.id, &document.id)
                .await
                .expect("read document from owner")
                .id,
            document.id
        );
        assert!(matches!(
            repo.get_document_in_kb(&second_kb.id, &document.id).await,
            Err(sqlx::Error::RowNotFound)
        ));
    }

    #[tokio::test]
    async fn source_document_count_and_snapshot_path_persist_across_attempts() {
        let pool = migrated_pool().await;
        let repo = KbRepository::new(pool);
        let kb = repo
            .create_kb(&CreateKbInput {
                name: "source retry".to_string(),
                description: None,
                embedding_model: None,
                embedding_channel_id: None,
            })
            .await
            .expect("create knowledge base");
        let source = repo
            .create_source(
                &kb.id,
                "url",
                Some("https://example.com/source"),
                None,
                None,
            )
            .await
            .expect("create source");
        let first = repo
            .create_document_with_source(
                &kb.id,
                "first.md",
                Some("old.snapshot"),
                "markdown",
                5,
                "source-hash-1",
                Some(&source.id),
                "url",
                Some("https://example.com/source"),
                None,
            )
            .await
            .expect("create first source document");
        repo.create_document_with_source(
            &kb.id,
            "second.md",
            Some("second.snapshot"),
            "markdown",
            6,
            "source-hash-2",
            Some(&source.id),
            "url",
            Some("https://example.com/source"),
            None,
        )
        .await
        .expect("create second source document");

        repo.update_document_snapshot_path(
            &kb.id,
            &source.id,
            &first.id,
            "replacement.snapshot",
        )
        .await
        .expect("replace retry snapshot path");

        assert_eq!(repo.count_documents_by_source(&source.id).await.unwrap(), 2);
        assert_eq!(
            repo.get_document(&first.id)
                .await
                .expect("read reused document")
                .file_path
                .as_deref(),
            Some("replacement.snapshot")
        );
    }
}
