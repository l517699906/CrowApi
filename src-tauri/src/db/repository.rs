use super::models::*;
use crate::core::secret_store::{
    api_key_lookup, default_secret_store, hash_api_key, key_preview_parts, verify_api_key,
    EncryptedSecret, SecretStore,
};
use sqlx::{
    sqlite::SqliteQueryResult, QueryBuilder, Sqlite, SqlitePool, Transaction,
};
use std::sync::Arc;

fn require_single_row(result: SqliteQueryResult) -> Result<(), sqlx::Error> {
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(sqlx::Error::RowNotFound)
    }
}

fn secret_error(error: impl Into<String>) -> sqlx::Error {
    sqlx::Error::Protocol(error.into())
}

#[derive(sqlx::FromRow)]
struct StoredSecret {
    version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

struct PreparedChannelSecret {
    secret_ref: String,
    last_four: String,
    encrypted: EncryptedSecret,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SecretMigrationReport {
    pub channels_migrated: usize,
    pub api_keys_migrated: usize,
}

pub struct Repository {
    pool: SqlitePool,
    secrets: Arc<dyn SecretStore>,
}

impl Repository {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            secrets: default_secret_store(),
        }
    }

    #[cfg(test)]
    fn with_secret_store(pool: SqlitePool, secrets: Arc<dyn SecretStore>) -> Self {
        Self { pool, secrets }
    }

    fn prepare_channel_secret(
        &self,
        channel_id: &str,
        plaintext: &str,
    ) -> Result<PreparedChannelSecret, sqlx::Error> {
        let secret_ref = format!("channel:{}", channel_id);
        let encrypted = self
            .secrets
            .encrypt(&secret_ref, plaintext)
            .map_err(secret_error)?;
        let (_, last_four) = key_preview_parts(plaintext);
        Ok(PreparedChannelSecret {
            secret_ref,
            last_four,
            encrypted,
        })
    }

    async fn upsert_channel_secret(
        transaction: &mut Transaction<'_, Sqlite>,
        channel_id: &str,
        prepared: &PreparedChannelSecret,
        now: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO secure_secrets (owner_type, owner_id, version, nonce, ciphertext, last_four, created_at, updated_at)
             VALUES ('channel', ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(owner_type, owner_id) DO UPDATE SET
                 version = excluded.version,
                 nonce = excluded.nonce,
                 ciphertext = excluded.ciphertext,
                 last_four = excluded.last_four,
                 updated_at = excluded.updated_at",
        )
        .bind(channel_id)
        .bind(prepared.encrypted.version)
        .bind(&prepared.encrypted.nonce)
        .bind(&prepared.encrypted.ciphertext)
        .bind(&prepared.last_four)
        .bind(now)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    async fn hydrate_channel_secret(&self, mut channel: Channel) -> Result<Channel, sqlx::Error> {
        let Some(secret_ref) = channel.secret_ref.as_deref() else {
            return Ok(channel);
        };
        let stored = sqlx::query_as::<_, StoredSecret>(
            "SELECT version, nonce, ciphertext FROM secure_secrets WHERE owner_type = 'channel' AND owner_id = ?",
        )
        .bind(&channel.id)
        .fetch_one(&self.pool)
        .await?;
        channel.api_key = self
            .secrets
            .decrypt(
                secret_ref,
                stored.version,
                &stored.nonce,
                &stored.ciphertext,
            )
            .map_err(secret_error)?;
        Ok(channel)
    }

    // ==================== Channel ====================

    pub async fn get_all_channels(&self) -> Result<Vec<Channel>, sqlx::Error> {
        let channels = sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY priority DESC, created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        let mut hydrated = Vec::with_capacity(channels.len());
        for channel in channels {
            hydrated.push(self.hydrate_channel_secret(channel).await?);
        }
        Ok(hydrated)
    }

    pub async fn get_channel(&self, id: &str) -> Result<Channel, sqlx::Error> {
        let channel = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        self.hydrate_channel_secret(channel).await
    }

    pub async fn get_enabled_channels(&self) -> Result<Vec<Channel>, sqlx::Error> {
        let channels = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE status = 1 ORDER BY priority DESC, weight DESC")
            .fetch_all(&self.pool)
            .await?;
        let mut hydrated = Vec::with_capacity(channels.len());
        for channel in channels {
            hydrated.push(self.hydrate_channel_secret(channel).await?);
        }
        Ok(hydrated)
    }

    pub async fn create_channel(&self, input: &CreateChannelInput) -> Result<Channel, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        let models = serde_json::to_string(&input.models).unwrap_or_else(|_| "[]".to_string());
        let config = input.config.as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "{}".to_string());
        let model_mapping = input.model_mapping.as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "{}".to_string());
        let prepared = if input.api_key.is_empty() {
            None
        } else {
            Some(self.prepare_channel_secret(&id, &input.api_key)?)
        };
        let stored_api_key = prepared
            .as_ref()
            .map(|_| format!("secret:{}", id))
            .unwrap_or_default();
        let secret_ref = prepared.as_ref().map(|secret| secret.secret_ref.as_str());
        let api_key_last4 = prepared
            .as_ref()
            .map(|secret| secret.last_four.as_str())
            .unwrap_or("");

        let mut transaction = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO channels (id, name, type, base_url, api_key, secret_ref, api_key_last4, models, status, priority, weight, config, model_mapping, timeout_secs, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.channel_type)
        .bind(&input.base_url)
        .bind(&stored_api_key)
        .bind(secret_ref)
        .bind(api_key_last4)
        .bind(&models)
        .bind(input.priority.unwrap_or(0))
        .bind(input.weight.unwrap_or(1))
        .bind(&config)
        .bind(&model_mapping)
        .bind(input.timeout_secs.unwrap_or(60).clamp(1, 3_600))
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;

        if let Some(prepared) = &prepared {
            Self::upsert_channel_secret(&mut transaction, &id, prepared, &now).await?;
        }
        transaction.commit().await?;

        self.get_channel(&id).await
    }

    pub async fn update_channel(&self, input: &UpdateChannelInput) -> Result<Channel, sqlx::Error> {
        let now = now_iso();

        let prepared = match input.api_key.as_deref() {
            Some(secret) if !secret.is_empty() => {
                Some(self.prepare_channel_secret(&input.id, secret)?)
            }
            _ => None,
        };
        let mut transaction = self.pool.begin().await?;
        if let Some(prepared) = &prepared {
            Self::upsert_channel_secret(&mut transaction, &input.id, prepared, &now).await?;
        }

        let mut q = sqlx::QueryBuilder::new("UPDATE channels SET updated_at = ");

        q.push_bind(now);

        if let Some(name) = &input.name {
            q.push(", name = ").push_bind(name);
        }
        if let Some(ct) = &input.channel_type {
            q.push(", type = ").push_bind(ct);
        }
        if let Some(base_url) = &input.base_url {
            q.push(", base_url = ").push_bind(base_url);
        }
        if let Some(prepared) = &prepared {
            q.push(", api_key = ")
                .push_bind(format!("secret:{}", input.id));
            q.push(", secret_ref = ").push_bind(&prepared.secret_ref);
            q.push(", api_key_last4 = ")
                .push_bind(&prepared.last_four);
        }
        if let Some(models) = &input.models {
            let m = serde_json::to_string(models).unwrap_or_else(|_| "[]".to_string());
            q.push(", models = ").push_bind(m);
        }
        if let Some(status) = input.status {
            q.push(", status = ").push_bind(status);
        }
        if let Some(priority) = input.priority {
            q.push(", priority = ").push_bind(priority);
        }
        if let Some(weight) = input.weight {
            q.push(", weight = ").push_bind(weight);
        }
        if let Some(config) = &input.config {
            let c = serde_json::to_string(config).unwrap_or_else(|_| "{}".to_string());
            q.push(", config = ").push_bind(c);
        }
        if let Some(mapping) = &input.model_mapping {
            let m = serde_json::to_string(mapping).unwrap_or_else(|_| "{}".to_string());
            q.push(", model_mapping = ").push_bind(m);
        }
        if let Some(timeout_secs) = input.timeout_secs {
            q.push(", timeout_secs = ").push_bind(timeout_secs.clamp(1, 3_600));
        }

        q.push(" WHERE id = ").push_bind(&input.id);
        require_single_row(q.build().execute(&mut *transaction).await?)?;
        transaction.commit().await?;

        self.get_channel(&input.id).await
    }

    pub async fn reorder_channels(&self, ordered_ids: &[String]) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let now = now_iso();
        let total = ordered_ids.len() as i64;

        for (index, id) in ordered_ids.iter().enumerate() {
            let result = sqlx::query(
                "UPDATE channels SET priority = ?, updated_at = ? WHERE id = ?",
            )
            .bind(total - index as i64)
            .bind(&now)
            .bind(id)
            .execute(&mut *transaction)
            .await?;

            if result.rows_affected() != 1 {
                return Err(sqlx::Error::RowNotFound);
            }
        }

        transaction.commit().await
    }

    pub async fn update_channel_status(&self, id: &str, status: i64) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query("UPDATE channels SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        require_single_row(result)
    }

    pub async fn delete_channel(&self, id: &str) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        require_single_row(result)?;
        sqlx::query("DELETE FROM secure_secrets WHERE owner_type = 'channel' AND owner_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await
    }

    pub async fn update_channel_test_result(&self, id: &str, ok: bool) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query("UPDATE channels SET last_test_at = ?, last_test_ok = ? WHERE id = ?")
            .bind(&now)
            .bind(if ok { 1 } else { 0 })
            .bind(id)
            .execute(&self.pool)
            .await?;
        require_single_row(result)
    }

    // ==================== API Key ====================

    pub async fn migrate_legacy_secrets(&self) -> Result<SecretMigrationReport, sqlx::Error> {
        let mut report = SecretMigrationReport::default();
        let legacy_channels = sqlx::query_as::<_, (String, String)>(
            "SELECT id, api_key FROM channels WHERE secret_ref IS NULL AND api_key != ''",
        )
        .fetch_all(&self.pool)
        .await?;

        for (id, plaintext) in legacy_channels {
            let prepared = self.prepare_channel_secret(&id, &plaintext)?;
            let now = now_iso();
            let mut transaction = self.pool.begin().await?;
            Self::upsert_channel_secret(&mut transaction, &id, &prepared, &now).await?;
            let result = sqlx::query(
                "UPDATE channels SET api_key = ?, secret_ref = ?, api_key_last4 = ?, updated_at = ? WHERE id = ? AND secret_ref IS NULL",
            )
            .bind(format!("secret:{}", id))
            .bind(&prepared.secret_ref)
            .bind(&prepared.last_four)
            .bind(&now)
            .bind(&id)
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() == 1 {
                transaction.commit().await?;
                report.channels_migrated += 1;
            }
        }

        let legacy_api_keys = sqlx::query_as::<_, (String, String)>(
            "SELECT id, key FROM api_keys WHERE key_hash IS NULL AND key != ''",
        )
        .fetch_all(&self.pool)
        .await?;
        for (id, plaintext) in legacy_api_keys {
            self.persist_api_key_digest(&id, &plaintext).await?;
            report.api_keys_migrated += 1;
        }

        Ok(report)
    }

    async fn persist_api_key_digest(&self, id: &str, plaintext: &str) -> Result<(), sqlx::Error> {
        let lookup = api_key_lookup(plaintext);
        let encoded_hash = hash_api_key(plaintext).map_err(secret_error)?;
        let (prefix, last_four) = key_preview_parts(plaintext);
        let now = now_iso();
        let result = sqlx::query(
            "UPDATE api_keys SET key = ?, key_lookup = ?, key_hash = ?, key_prefix = ?, key_last4 = ?, updated_at = ? WHERE id = ?",
        )
        .bind(format!("redacted:{}", id))
        .bind(lookup)
        .bind(encoded_hash)
        .bind(prefix)
        .bind(last_four)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        require_single_row(result)
    }

    pub async fn get_all_api_keys(&self) -> Result<Vec<ApiKey>, sqlx::Error> {
        sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_api_key_by_key(&self, key: &str) -> Result<ApiKey, sqlx::Error> {
        let lookup = api_key_lookup(key);
        if let Some(record) = sqlx::query_as::<_, ApiKey>(
            "SELECT * FROM api_keys WHERE key_lookup = ? AND status = 1",
        )
        .bind(&lookup)
        .fetch_optional(&self.pool)
        .await?
        {
            if record
                .key_hash
                .as_deref()
                .is_some_and(|encoded| verify_api_key(key, encoded))
            {
                return Ok(record);
            }
            return Err(sqlx::Error::RowNotFound);
        }

        let legacy = sqlx::query_as::<_, ApiKey>(
            "SELECT * FROM api_keys WHERE key = ? AND status = 1 AND key_hash IS NULL",
        )
        .bind(key)
        .fetch_one(&self.pool)
        .await?;
        self.persist_api_key_digest(&legacy.id, key).await?;
        sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE id = ?")
            .bind(&legacy.id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn create_api_key(&self, input: &CreateApiKeyInput) -> Result<ApiKey, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_iso();
        let key = format!("sk-crowapi-{}", uuid::Uuid::new_v4().simple());
        let lookup = api_key_lookup(&key);
        let encoded_hash = hash_api_key(&key).map_err(secret_error)?;
        let (prefix, last_four) = key_preview_parts(&key);
        let allowed_models = serde_json::to_string(&input.allowed_models.clone().unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());
        let allowed_channels = serde_json::to_string(&input.allowed_channels.clone().unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());

        sqlx::query(
            "INSERT INTO api_keys (id, name, key, key_lookup, key_hash, key_prefix, key_last4, status, allowed_models, allowed_channels, quota_limit, quota_used, expires_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, 0, ?, ?, ?)"
        )
        .bind(&id)
        .bind(&input.name)
        .bind(format!("redacted:{}", id))
        .bind(&lookup)
        .bind(&encoded_hash)
        .bind(&prefix)
        .bind(&last_four)
        .bind(&allowed_models)
        .bind(&allowed_channels)
        .bind(input.quota_limit.unwrap_or(-1))
        .bind(&input.expires_at)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        let mut created = sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        created.key = key;
        Ok(created)
    }

    pub async fn update_api_key_status(&self, id: &str, status: i64) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query("UPDATE api_keys SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        require_single_row(result)
    }

    pub async fn update_api_key_quota(&self, id: &str, quota_limit: i64) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query("UPDATE api_keys SET quota_limit = ?, updated_at = ? WHERE id = ?")
            .bind(quota_limit)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        require_single_row(result)
    }

    pub async fn update_api_key_expiration(
        &self,
        id: &str,
        expires_at: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let now = now_iso();
        let result = sqlx::query("UPDATE api_keys SET expires_at = ?, updated_at = ? WHERE id = ?")
            .bind(expires_at)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        require_single_row(result)
    }

    pub async fn delete_api_key(&self, id: &str) -> Result<(), sqlx::Error> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        require_single_row(result)
    }

    pub async fn increment_quota(&self, id: &str, tokens: i64) -> Result<(), sqlx::Error> {
        let result = sqlx::query("UPDATE api_keys SET quota_used = quota_used + ? WHERE id = ?")
            .bind(tokens)
            .bind(id)
            .execute(&self.pool)
            .await?;
        require_single_row(result)
    }

    pub async fn get_total_quota_used(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COALESCE(SUM(quota_used), 0) FROM api_keys")
            .fetch_one(&self.pool)
            .await
    }

    // ==================== Request Log ====================

    pub async fn create_log(&self, log: &RequestLog) -> Result<i64, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO request_logs (id, api_key_id, api_key_name, channel_id, channel_name, model, upstream_model, mode, status_code, prompt_tokens, completion_tokens, total_tokens, duration_ms, error_message, is_stream, is_retry, created_at, request_body, response_choices, risk_level, risk_score, risk_summary, security_action, sanitized, blocked_reason, trace_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&log.id)
        .bind(&log.api_key_id)
        .bind(&log.api_key_name)
        .bind(&log.channel_id)
        .bind(&log.channel_name)
        .bind(&log.model)
        .bind(&log.upstream_model)
        .bind(&log.mode)
        .bind(log.status_code)
        .bind(log.prompt_tokens)
        .bind(log.completion_tokens)
        .bind(log.total_tokens)
        .bind(log.duration_ms)
        .bind(&log.error_message)
        .bind(log.is_stream)
        .bind(log.is_retry)
        .bind(&log.created_at)
        .bind(&log.request_body)
        .bind(&log.response_choices)
        .bind(&log.risk_level)
        .bind(log.risk_score)
        .bind(&log.risk_summary)
        .bind(&log.security_action)
        .bind(log.sanitized)
        .bind(&log.blocked_reason)
        .bind(&log.trace_id)
        .execute(&self.pool)
        .await?;
        let seq = result.last_insert_rowid();

        // Backfill seq with rowid for new rows
        sqlx::query(
            "UPDATE request_logs SET seq = ? WHERE id = ? AND (seq = 0 OR seq IS NULL)",
        )
        .bind(seq)
        .bind(&log.id)
        .execute(&self.pool)
        .await?;
        Ok(seq)
    }

    pub async fn create_security_findings(&self, log_id: &str, findings: &[crate::security::SecurityFinding], action: &str) -> Result<(), sqlx::Error> {
        for finding in findings {
            sqlx::query(
                "INSERT INTO request_security_findings (id, log_id, phase, category, rule_id, severity, title, description, location, evidence_masked, evidence_hash, action, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(crate::utils::id::new_id())
            .bind(log_id)
            .bind(&finding.phase)
            .bind(&finding.category)
            .bind(&finding.rule_id)
            .bind(finding.severity.as_str())
            .bind(&finding.title)
            .bind(&finding.description)
            .bind(&finding.location)
            .bind(&finding.evidence_masked)
            .bind(Option::<String>::None)
            .bind(action)
            .bind(crate::utils::time::now_iso())
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn get_security_findings(&self, log_id: &str) -> Result<Vec<RequestSecurityFinding>, sqlx::Error> {
        sqlx::query_as::<_, RequestSecurityFinding>("SELECT * FROM request_security_findings WHERE log_id = ? ORDER BY created_at ASC")
            .bind(log_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_log(&self, id: &str) -> Result<RequestLog, sqlx::Error> {
        sqlx::query_as::<_, RequestLog>("SELECT * FROM request_logs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn delete_logs_before(&self, before_date: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM request_logs WHERE created_at < ?")
            .bind(before_date)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_all_logs(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM request_logs")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_log(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM request_logs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_logs(&self, limit: i64, offset: i64) -> Result<Vec<RequestLog>, sqlx::Error> {
        sqlx::query_as::<_, RequestLog>(
            "SELECT * FROM request_logs ORDER BY rowid DESC LIMIT ? OFFSET ?"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_logs_after(&self, after_seq: i64, limit: i64) -> Result<Vec<RequestLog>, sqlx::Error> {
        sqlx::query_as::<_, RequestLog>(
            "SELECT * FROM request_logs WHERE rowid > ? ORDER BY rowid ASC LIMIT ?",
        )
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn search_logs(
        &self,
        keyword: Option<&str>,
        api_key_name: Option<&str>,
        channel_name: Option<&str>,
        model: Option<&str>,
        date_from: Option<&str>,
        date_to: Option<&str>,
        trace_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<RequestLog>, sqlx::Error> {
        let mut q = QueryBuilder::new("SELECT * FROM request_logs WHERE 1=1");
        push_log_filters(
            &mut q,
            keyword,
            api_key_name,
            channel_name,
            model,
            date_from,
            date_to,
            trace_id,
        );

        q.push(" ORDER BY request_logs.rowid DESC LIMIT ").push_bind(limit);
        q.push(" OFFSET ").push_bind(offset);

        q.build_query_as::<RequestLog>().fetch_all(&self.pool).await
    }

    pub async fn search_logs_after(
        &self,
        keyword: Option<&str>,
        api_key_name: Option<&str>,
        channel_name: Option<&str>,
        model: Option<&str>,
        date_from: Option<&str>,
        date_to: Option<&str>,
        trace_id: Option<&str>,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<RequestLog>, sqlx::Error> {
        let mut q = QueryBuilder::new("SELECT * FROM request_logs WHERE request_logs.rowid > ");
        q.push_bind(after_seq);
        push_log_filters(
            &mut q,
            keyword,
            api_key_name,
            channel_name,
            model,
            date_from,
            date_to,
            trace_id,
        );
        q.push(" ORDER BY request_logs.rowid ASC LIMIT ").push_bind(limit);
        q.build_query_as::<RequestLog>().fetch_all(&self.pool).await
    }

    pub async fn get_dashboard_stats(
        &self,
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> Result<DashboardStats, sqlx::Error> {
        let mut today_query = QueryBuilder::new(
            "SELECT COUNT(*) AS request_count, COALESCE(SUM(total_tokens), 0) AS total_tokens, \
             COALESCE(AVG(duration_ms), CAST(0 AS REAL)) AS avg_latency FROM request_logs WHERE 1=1",
        );
        push_stats_date_range(&mut today_query, date_from, date_to);
        let (today_requests, today_total_tokens, avg_latency) = today_query
            .build_query_as::<(i64, i64, f64)>()
            .fetch_one(&self.pool)
            .await?;

        let active_channels: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channels WHERE status = 1"
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let total_channels: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let total_api_keys: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys")
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let total_requests: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM request_logs")
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let total_tokens: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(total_tokens), 0) FROM request_logs")
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let total_knowledge_bases: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kb_knowledge_bases")
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let total_kb_documents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kb_documents")
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let total_kb_chunks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kb_chunks")
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let protocols = self.get_protocol_usage(None, None).await?;

        Ok(DashboardStats {
            today_requests,
            today_total_tokens,
            active_channels,
            avg_latency_ms: avg_latency,
            total_channels,
            total_api_keys,
            total_requests,
            total_tokens,
            total_knowledge_bases,
            total_kb_documents,
            total_kb_chunks,
            protocols,
        })
    }

    pub async fn get_channel_stats(&self) -> Result<Vec<ChannelStats>, sqlx::Error> {
        sqlx::query_as::<_, ChannelStats>(
            "SELECT\n                r.channel_id as channel_id,\n                COUNT(*) as total_calls,\n                SUM(CASE WHEN r.status_code >= 200 AND r.status_code < 300 THEN 1 ELSE 0 END) as success_calls,\n                SUM(CASE WHEN r.status_code >= 200 AND r.status_code < 300 THEN 0 ELSE 1 END) as failed_calls,\n                COALESCE(SUM(r.total_tokens), 0) as total_tokens,\n                COALESCE(SUM(r.prompt_tokens), 0) as prompt_tokens,\n                COALESCE(SUM(r.completion_tokens), 0) as completion_tokens,\n                COALESCE(AVG(r.duration_ms), 0) as avg_latency_ms,\n                MAX(r.created_at) as last_call_at\n            FROM request_logs r\n            WHERE r.channel_id IS NOT NULL\n            GROUP BY r.channel_id"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_api_key_stats(&self) -> Result<Vec<ApiKeyStats>, sqlx::Error> {
        sqlx::query_as::<_, ApiKeyStats>(
            "SELECT\n                r.api_key_id as api_key_id,\n                COUNT(*) as total_calls,\n                SUM(CASE WHEN r.status_code >= 200 AND r.status_code < 300 THEN 1 ELSE 0 END) as success_calls,\n                SUM(CASE WHEN r.status_code >= 200 AND r.status_code < 300 THEN 0 ELSE 1 END) as failed_calls,\n                COALESCE(SUM(r.total_tokens), 0) as total_tokens,\n                COALESCE(SUM(r.prompt_tokens), 0) as prompt_tokens,\n                COALESCE(SUM(r.completion_tokens), 0) as completion_tokens,\n                COALESCE(AVG(r.duration_ms), 0) as avg_latency_ms,\n                MAX(r.created_at) as last_call_at\n            FROM request_logs r\n            WHERE r.api_key_id IS NOT NULL\n            GROUP BY r.api_key_id"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_usage_stats(
        &self,
        date_from: Option<&str>,
        date_to: Option<&str>,
        bucket_seconds: i64,
        bucket_count: i64,
    ) -> Result<UsageStats, sqlx::Error> {
        let mut query = QueryBuilder::new(
            "SELECT COUNT(*) AS total_requests, COALESCE(SUM(total_tokens), 0) AS total_tokens, \
             COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0) AS failed_requests \
             FROM request_logs WHERE 1=1",
        );
        push_stats_date_range(&mut query, date_from, date_to);
        let (total_requests, total_tokens, failed_requests) = query
            .build_query_as::<(i64, i64, i64)>()
            .fetch_one(&self.pool)
            .await?;
        let protocols = self.get_protocol_usage(date_from, date_to).await?;
        let series = self
            .get_usage_series(date_from, date_to, bucket_seconds, bucket_count)
            .await?;
        let models = self.get_model_usage(date_from, date_to).await?;
        let channels = self.get_channel_usage(date_from, date_to).await?;

        Ok(UsageStats {
            total_requests,
            total_tokens,
            failed_requests,
            protocols,
            series,
            models,
            channels,
        })
    }

    pub async fn get_protocol_usage(
        &self,
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> Result<Vec<ProtocolUsageStat>, sqlx::Error> {
        let mut query = QueryBuilder::new(
            "SELECT mode, COUNT(*) AS request_count, COALESCE(SUM(total_tokens), 0) AS total_tokens \
             FROM request_logs WHERE 1=1",
        );
        push_stats_date_range(&mut query, date_from, date_to);
        query.push(" GROUP BY mode ORDER BY request_count DESC, mode ASC");
        query
            .build_query_as::<ProtocolUsageStat>()
            .fetch_all(&self.pool)
            .await
    }

    async fn get_usage_series(
        &self,
        date_from: Option<&str>,
        date_to: Option<&str>,
        bucket_seconds: i64,
        bucket_count: i64,
    ) -> Result<Vec<UsageBucketStat>, sqlx::Error> {
        let Some(from) = date_from else {
            return Ok(Vec::new());
        };

        let mut query = QueryBuilder::new("SELECT CAST((unixepoch(created_at) - unixepoch(");
        query.push_bind(from.to_string());
        query.push(")) / ").push_bind(bucket_seconds);
        query.push(
            " AS INTEGER) AS bucket_index, COUNT(*) AS request_count \
             FROM request_logs WHERE 1=1",
        );
        push_stats_date_range(&mut query, date_from, date_to);
        query.push(" GROUP BY bucket_index HAVING bucket_index >= 0 AND bucket_index < ");
        query.push_bind(bucket_count);
        query.push(" ORDER BY bucket_index ASC");
        query
            .build_query_as::<UsageBucketStat>()
            .fetch_all(&self.pool)
            .await
    }

    async fn get_model_usage(
        &self,
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> Result<Vec<ModelUsageStat>, sqlx::Error> {
        let mut query = QueryBuilder::new(
            "SELECT model AS name, COUNT(*) AS request_count, \
             COALESCE(SUM(total_tokens), 0) AS total_tokens \
             FROM request_logs WHERE 1=1",
        );
        push_stats_date_range(&mut query, date_from, date_to);
        query.push(" GROUP BY model ORDER BY total_tokens DESC, request_count DESC, name ASC");
        query
            .build_query_as::<ModelUsageStat>()
            .fetch_all(&self.pool)
            .await
    }

    async fn get_channel_usage(
        &self,
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> Result<Vec<ChannelUsageStat>, sqlx::Error> {
        let mut query = QueryBuilder::new(
            "SELECT COALESCE(request_logs.channel_id, request_logs.channel_name, 'unassigned') AS id, \
             COALESCE(request_logs.channel_name, '未分配渠道') AS name, \
             COALESCE(channels.type, 'custom') AS channel_type, COUNT(*) AS request_count \
             FROM request_logs LEFT JOIN channels ON channels.id = request_logs.channel_id \
             WHERE 1=1",
        );
        push_stats_date_range_qualified(&mut query, date_from, date_to);
        query.push(
            " GROUP BY request_logs.channel_id, request_logs.channel_name, channels.type \
             ORDER BY request_count DESC, name ASC",
        );
        query
            .build_query_as::<ChannelUsageStat>()
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_log_stats(&self, days: i64) -> Result<Vec<LogStats>, sqlx::Error> {
        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(days))
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();

        sqlx::query_as::<_, LogStats>(
            "SELECT substr(created_at, 1, 10) as date, COUNT(*) as count, COALESCE(SUM(total_tokens), 0) as total_tokens
             FROM request_logs
             WHERE created_at >= ?
             GROUP BY date
             ORDER BY date DESC"
        )
        .bind(&since)
        .fetch_all(&self.pool)
        .await
    }
}

fn push_date_range(
    query: &mut QueryBuilder<'_, Sqlite>,
    date_from: Option<&str>,
    date_to: Option<&str>,
) {
    if let Some(from) = date_from {
        query.push(" AND created_at >= ").push_bind(from.to_string());
    }
    if let Some(to) = date_to {
        query.push(" AND created_at <= ").push_bind(to.to_string());
    }
}

fn push_stats_date_range(
    query: &mut QueryBuilder<'_, Sqlite>,
    date_from: Option<&str>,
    date_to: Option<&str>,
) {
    if let Some(from) = date_from {
        query.push(" AND created_at >= ").push_bind(from.to_string());
    }
    if let Some(to) = date_to {
        query.push(" AND created_at < ").push_bind(to.to_string());
    }
}

fn push_stats_date_range_qualified(
    query: &mut QueryBuilder<'_, Sqlite>,
    date_from: Option<&str>,
    date_to: Option<&str>,
) {
    if let Some(from) = date_from {
        query
            .push(" AND request_logs.created_at >= ")
            .push_bind(from.to_string());
    }
    if let Some(to) = date_to {
        query
            .push(" AND request_logs.created_at < ")
            .push_bind(to.to_string());
    }
}

fn push_log_filters(
    query: &mut QueryBuilder<'_, Sqlite>,
    keyword: Option<&str>,
    api_key_name: Option<&str>,
    channel_name: Option<&str>,
    model: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
    trace_id: Option<&str>,
) {
    if let Some(keyword) = keyword {
        let pattern = format!("%{}%", keyword);
        query.push(" AND (api_key_name LIKE ").push_bind(pattern.clone());
        query.push(" OR channel_name LIKE ").push_bind(pattern.clone());
        query.push(" OR model LIKE ").push_bind(pattern.clone());
        query.push(" OR upstream_model LIKE ").push_bind(pattern.clone());
        query.push(" OR api_key_id LIKE ").push_bind(pattern.clone());
        query.push(" OR trace_id LIKE ").push_bind(pattern.clone());
        query.push(" OR id LIKE ").push_bind(pattern);
        query.push(")");
    }

    if let Some(name) = api_key_name {
        query.push(" AND api_key_name LIKE ").push_bind(format!("%{}%", name));
    }
    if let Some(name) = channel_name {
        query.push(" AND channel_name LIKE ").push_bind(format!("%{}%", name));
    }
    if let Some(model) = model {
        let pattern = format!("%{}%", model);
        query.push(" AND (model LIKE ").push_bind(pattern.clone());
        query.push(" OR upstream_model LIKE ").push_bind(pattern);
        query.push(")");
    }
    if let Some(trace_id) = trace_id {
        query.push(" AND trace_id LIKE ").push_bind(format!("%{}%", trace_id));
    }
    push_date_range(query, date_from, date_to);
}

#[cfg(test)]
mod tests {
    use super::{Repository, SecretMigrationReport};
    use crate::core::secret_store::{EncryptedSecret, SecretStore};
    use crate::db::models::{CreateApiKeyInput, CreateChannelInput, UpdateChannelInput};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    struct TestSecretStore;

    struct FailingSecretStore;

    impl SecretStore for FailingSecretStore {
        fn encrypt(&self, _context: &str, _plaintext: &str) -> Result<EncryptedSecret, String> {
            Err("system keychain unavailable".to_string())
        }

        fn decrypt(
            &self,
            _context: &str,
            _version: i64,
            _nonce: &[u8],
            _ciphertext: &[u8],
        ) -> Result<String, String> {
            Err("system keychain unavailable".to_string())
        }
    }

    impl SecretStore for TestSecretStore {
        fn encrypt(&self, context: &str, plaintext: &str) -> Result<EncryptedSecret, String> {
            let mut ciphertext = context.as_bytes().to_vec();
            ciphertext.push(0);
            ciphertext.extend(plaintext.bytes().map(|byte| byte ^ 0xA5));
            Ok(EncryptedSecret {
                version: 1,
                nonce: vec![0; 24],
                ciphertext,
            })
        }

        fn decrypt(
            &self,
            context: &str,
            version: i64,
            _nonce: &[u8],
            ciphertext: &[u8],
        ) -> Result<String, String> {
            if version != 1 {
                return Err("unsupported version".to_string());
            }
            let prefix = [context.as_bytes(), &[0]].concat();
            let encrypted = ciphertext
                .strip_prefix(prefix.as_slice())
                .ok_or_else(|| "context mismatch".to_string())?;
            String::from_utf8(encrypted.iter().map(|byte| byte ^ 0xA5).collect())
                .map_err(|error| error.to_string())
        }
    }

    fn test_repository(pool: sqlx::SqlitePool) -> Repository {
        Repository::with_secret_store(pool, Arc::new(TestSecretStore))
    }

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create repository test database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        pool
    }

    #[tokio::test]
    async fn dashboard_stats_return_real_zero_for_an_empty_date_range() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create in-memory SQLite pool");

        for statement in [
            "CREATE TABLE request_logs (mode TEXT NOT NULL, total_tokens INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)",
            "CREATE TABLE channels (status INTEGER NOT NULL)",
            "CREATE TABLE api_keys (id TEXT)",
            "CREATE TABLE kb_knowledge_bases (id TEXT)",
            "CREATE TABLE kb_documents (id TEXT)",
            "CREATE TABLE kb_chunks (id TEXT)",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("create dashboard statistics table");
        }

        sqlx::query(
            "INSERT INTO request_logs (mode, total_tokens, duration_ms, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind("chat")
        .bind(42_i64)
        .bind(250_i64)
        .bind("2026-08-19T12:00:00.000Z")
        .execute(&pool)
        .await
        .expect("insert historical request log");

        let repository = Repository::new(pool);
        let stats = repository
            .get_dashboard_stats(
                Some("2026-08-20T00:00:00.000Z"),
                Some("2026-08-21T00:00:00.000Z"),
            )
            .await
            .expect("load dashboard statistics for an empty date range");

        assert_eq!(stats.today_requests, 0);
        assert_eq!(stats.today_total_tokens, 0);
        assert_eq!(stats.avg_latency_ms, 0.0);
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.total_tokens, 42);
    }

    #[tokio::test]
    async fn api_key_expiration_is_persisted_updated_and_cleared() {
        let pool = migrated_pool().await;
        let repository = Repository::new(pool);
        let created = repository
            .create_api_key(&CreateApiKeyInput {
                name: "expiring key".to_string(),
                allowed_models: Some(vec!["gpt-4o".to_string()]),
                allowed_channels: Some(vec!["channel-1".to_string()]),
                quota_limit: Some(100),
                expires_at: Some("2026-09-01T00:00:00Z".to_string()),
            })
            .await
            .expect("create expiring API key");
        assert_eq!(created.expires_at.as_deref(), Some("2026-09-01T00:00:00Z"));

        repository
            .update_api_key_expiration(&created.id, Some("2026-10-01T00:00:00Z"))
            .await
            .expect("update API key expiration");
        let updated = repository
            .get_api_key_by_key(&created.key)
            .await
            .expect("read updated API key");
        assert_eq!(updated.expires_at.as_deref(), Some("2026-10-01T00:00:00Z"));

        repository
            .update_api_key_expiration(&created.id, None)
            .await
            .expect("clear API key expiration");
        let cleared = repository
            .get_api_key_by_key(&created.key)
            .await
            .expect("read API key with cleared expiration");
        assert!(cleared.expires_at.is_none());
    }

    #[tokio::test]
    async fn mutations_report_missing_targets() {
        let repository = Repository::new(migrated_pool().await);

        assert!(matches!(
            repository.update_channel_status("missing-channel", 1).await,
            Err(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            repository
                .update_channel(&UpdateChannelInput {
                    id: "missing-channel".to_string(),
                    name: Some("renamed".to_string()),
                    channel_type: None,
                    base_url: None,
                    api_key: None,
                    models: None,
                    status: None,
                    priority: None,
                    weight: None,
                    config: None,
                    model_mapping: None,
                    timeout_secs: None,
                })
                .await,
            Err(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            repository.delete_channel("missing-channel").await,
            Err(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            repository.update_api_key_status("missing-key", 0).await,
            Err(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            repository.update_api_key_quota("missing-key", 1).await,
            Err(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            repository
                .update_api_key_expiration("missing-key", None)
                .await,
            Err(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            repository.delete_api_key("missing-key").await,
            Err(sqlx::Error::RowNotFound)
        ));
    }

    #[tokio::test]
    async fn channel_secret_is_encrypted_and_hydrated() {
        let pool = migrated_pool().await;
        let repository = test_repository(pool.clone());
        let created = repository
            .create_channel(&CreateChannelInput {
                name: "encrypted channel".to_string(),
                channel_type: "openai".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: "sk-provider-secret".to_string(),
                models: vec!["gpt-test".to_string()],
                priority: None,
                weight: None,
                config: None,
                model_mapping: None,
                timeout_secs: None,
            })
            .await
            .expect("create encrypted channel");

        assert_eq!(created.api_key, "sk-provider-secret");
        assert_eq!(created.api_key_last4, "cret");
        let stored_marker: String = sqlx::query_scalar("SELECT api_key FROM channels WHERE id = ?")
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .expect("read channel marker");
        let ciphertext: Vec<u8> = sqlx::query_scalar(
            "SELECT ciphertext FROM secure_secrets WHERE owner_type = 'channel' AND owner_id = ?",
        )
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .expect("read encrypted channel secret");
        assert!(stored_marker.starts_with("secret:"));
        assert_ne!(ciphertext, b"sk-provider-secret");
        assert!(!stored_marker.contains("provider-secret"));
    }

    #[tokio::test]
    async fn channel_secret_rotation_and_delete_keep_vault_in_sync() {
        let pool = migrated_pool().await;
        let repository = test_repository(pool.clone());
        let created = repository
            .create_channel(&CreateChannelInput {
                name: "rotating channel".to_string(),
                channel_type: "openai".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: "sk-provider-old".to_string(),
                models: vec!["gpt-test".to_string()],
                priority: None,
                weight: None,
                config: None,
                model_mapping: None,
                timeout_secs: None,
            })
            .await
            .expect("create channel for rotation");

        let updated = repository
            .update_channel(&UpdateChannelInput {
                id: created.id.clone(),
                name: None,
                channel_type: None,
                base_url: None,
                api_key: Some("sk-provider-new".to_string()),
                models: None,
                status: None,
                priority: None,
                weight: None,
                config: None,
                model_mapping: None,
                timeout_secs: None,
            })
            .await
            .expect("rotate channel secret");

        assert_eq!(updated.api_key, "sk-provider-new");
        assert_eq!(updated.api_key_last4, "-new");
        repository
            .delete_channel(&created.id)
            .await
            .expect("delete rotated channel");
        let secret_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM secure_secrets WHERE owner_type = 'channel' AND owner_id = ?",
        )
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .expect("count channel secrets after delete");
        assert_eq!(secret_count, 0);
    }

    #[tokio::test]
    async fn api_key_digest_authenticates_only_the_original_secret() {
        let pool = migrated_pool().await;
        let repository = Repository::new(pool.clone());
        let created = repository
            .create_api_key(&CreateApiKeyInput {
                name: "digest key".to_string(),
                allowed_models: None,
                allowed_channels: None,
                quota_limit: Some(0),
                expires_at: None,
            })
            .await
            .expect("create API key");

        assert_eq!(
            repository
                .get_api_key_by_key(&created.key)
                .await
                .expect("authenticate original API key")
                .id,
            created.id,
        );
        assert!(matches!(
            repository.get_api_key_by_key("sk-crowapi-wrong").await,
            Err(sqlx::Error::RowNotFound)
        ));
        let stored: (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT key, key_lookup, key_hash FROM api_keys WHERE id = ?",
        )
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .expect("read stored API key digest");
        assert!(stored.0.starts_with("redacted:"));
        assert!(!stored.0.contains(&created.key));
        assert_eq!(stored.1.as_deref().map(str::len), Some(64));
        assert!(stored.2.as_deref().is_some_and(|hash| hash.starts_with("$argon2")));
    }

    #[tokio::test]
    async fn legacy_secrets_migrate_without_breaking_authentication() {
        let pool = migrated_pool().await;
        sqlx::query(
            "INSERT INTO channels (id, name, type, base_url, api_key, models, status, priority, weight, config, model_mapping, timeout_secs, created_at, updated_at)
             VALUES ('legacy-channel', 'legacy', 'openai', 'https://api.example.com/v1', 'legacy-provider-key', '[]', 1, 0, 1, '{}', '{}', 60, 'now', 'now')",
        )
        .execute(&pool)
        .await
        .expect("insert legacy channel");
        sqlx::query(
            "INSERT INTO api_keys (id, name, key, status, allowed_models, allowed_channels, quota_limit, quota_used, created_at, updated_at)
             VALUES ('legacy-key', 'legacy', 'sk-crowapi-legacy-key', 1, '[]', '[]', 0, 0, 'now', 'now')",
        )
        .execute(&pool)
        .await
        .expect("insert legacy API key");
        let repository = test_repository(pool.clone());

        let report = repository
            .migrate_legacy_secrets()
            .await
            .expect("migrate legacy secrets");

        assert_eq!(report.channels_migrated, 1);
        assert_eq!(report.api_keys_migrated, 1);
        assert_eq!(
            repository
                .get_channel("legacy-channel")
                .await
                .expect("hydrate migrated channel")
                .api_key,
            "legacy-provider-key",
        );
        assert_eq!(
            repository
                .get_api_key_by_key("sk-crowapi-legacy-key")
                .await
                .expect("authenticate migrated API key")
                .id,
            "legacy-key",
        );
        let stored_channel: String =
            sqlx::query_scalar("SELECT api_key FROM channels WHERE id = 'legacy-channel'")
                .fetch_one(&pool)
                .await
                .expect("read migrated channel marker");
        let stored_api_key: String =
            sqlx::query_scalar("SELECT key FROM api_keys WHERE id = 'legacy-key'")
                .fetch_one(&pool)
                .await
                .expect("read migrated API key marker");
        assert!(stored_channel.starts_with("secret:"));
        assert!(stored_api_key.starts_with("redacted:"));
        assert!(!stored_channel.contains("provider-key"));
        assert!(!stored_api_key.contains("crowapi-legacy"));

        assert_eq!(
            repository
                .migrate_legacy_secrets()
                .await
                .expect("repeat legacy migration"),
            SecretMigrationReport::default(),
        );
    }

    #[tokio::test]
    async fn legacy_channel_migration_fails_closed_without_keychain() {
        let pool = migrated_pool().await;
        sqlx::query(
            "INSERT INTO channels (id, name, type, base_url, api_key, models, status, priority, weight, config, model_mapping, timeout_secs, created_at, updated_at)
             VALUES ('legacy-channel', 'legacy', 'openai', 'https://api.example.com/v1', 'legacy-provider-key', '[]', 1, 0, 1, '{}', '{}', 60, 'now', 'now')",
        )
        .execute(&pool)
        .await
        .expect("insert legacy channel");
        let repository = Repository::with_secret_store(pool.clone(), Arc::new(FailingSecretStore));

        assert!(repository.migrate_legacy_secrets().await.is_err());
        let stored: (String, Option<String>) =
            sqlx::query_as("SELECT api_key, secret_ref FROM channels WHERE id = 'legacy-channel'")
                .fetch_one(&pool)
                .await
                .expect("read unchanged legacy channel");
        assert_eq!(stored.0, "legacy-provider-key");
        assert!(stored.1.is_none());
    }
}
