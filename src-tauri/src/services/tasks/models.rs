use serde::{Deserialize, Serialize};

pub const TASK_CANCELLED: &str = "BACKGROUND_TASK_CANCELLED";

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTask {
    pub id: String,
    pub domain: String,
    pub task_type: String,
    pub resource_type: String,
    pub resource_id: String,
    pub subject_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub retry_of: Option<String>,
    pub status: String,
    pub stage: String,
    pub progress: i64,
    pub total_items: i64,
    pub done_items: i64,
    pub payload_json: String,
    pub result_json: Option<String>,
    pub error_message: Option<String>,
    pub retryable: i64,
    pub auto_resume: i64,
    pub cancel_requested: i64,
    pub attempt: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<String>,
    pub heartbeat_at: Option<String>,
    pub next_retry_at: Option<String>,
    pub max_attempts: i64,
    pub dead_letter: i64,
}

#[derive(Debug, Clone)]
pub struct TaskSpec {
    pub domain: String,
    pub task_type: String,
    pub resource_type: String,
    pub resource_id: String,
    pub subject_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub retry_of: Option<String>,
    pub payload: serde_json::Value,
    pub retryable: bool,
    pub auto_resume: bool,
    pub attempt: i64,
    pub total_items: i64,
    pub max_attempts: i64,
}

impl TaskSpec {
    pub fn new(
        domain: impl Into<String>,
        task_type: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
    ) -> Self {
        Self {
            domain: domain.into(),
            task_type: task_type.into(),
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            subject_id: None,
            parent_task_id: None,
            idempotency_key: None,
            retry_of: None,
            payload: serde_json::json!({}),
            retryable: true,
            auto_resume: false,
            attempt: 1,
            total_items: 0,
            max_attempts: 5,
        }
    }

    pub fn subject_id(mut self, subject_id: Option<String>) -> Self {
        self.subject_id = subject_id;
        self
    }

    pub fn parent_task_id(mut self, parent_task_id: Option<String>) -> Self {
        self.parent_task_id = parent_task_id;
        self
    }

    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn auto_resume(mut self, auto_resume: bool) -> Self {
        self.auto_resume = auto_resume;
        self
    }

    pub fn total_items(mut self, total_items: i64) -> Self {
        self.total_items = total_items.max(0);
        self
    }

    pub fn max_attempts(mut self, max_attempts: i64) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListFilter {
    pub domain: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTaskEvent<'a> {
    pub task_id: &'a str,
    pub domain: &'a str,
    pub resource_type: &'a str,
    pub resource_id: &'a str,
    pub subject_id: Option<&'a str>,
    pub parent_task_id: Option<&'a str>,
    pub status: &'a str,
    pub stage: &'a str,
    pub progress: i64,
    pub detail: Option<&'a str>,
}
