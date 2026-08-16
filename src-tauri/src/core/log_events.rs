use crate::db::models::RequestLog;
use crate::db::repository::Repository;
use crate::security::SecurityFinding;
use crate::AppState;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

const EVENT_NAME: &str = "request-logs-changed";
const FLUSH_INTERVAL_MS: u64 = 200;

#[derive(Debug, Clone, Serialize)]
pub struct LogChangedEvent {
    pub latest_seq: i64,
    pub pending: u64,
    pub reset: bool,
}

#[derive(Debug, Default)]
pub struct LogEventState {
    latest_seq: AtomicI64,
    pending: AtomicU64,
    dirty: AtomicBool,
    reset: AtomicBool,
}

impl LogEventState {
    pub fn spawn(self: Arc<Self>, app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(FLUSH_INTERVAL_MS));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // 启动时不立即刷新，保证事件频率始终受窗口限制。
            ticker.tick().await;

            loop {
                ticker.tick().await;
                if !self.dirty.swap(false, Ordering::AcqRel) {
                    continue;
                }

                let event = LogChangedEvent {
                    latest_seq: self.latest_seq.load(Ordering::Acquire),
                    pending: self.pending.swap(0, Ordering::AcqRel),
                    reset: self.reset.swap(false, Ordering::AcqRel),
                };

                if let Err(error) = app.emit_to("main", EVENT_NAME, event) {
                    tracing::debug!(%error, "failed to emit request log change event");
                }
            }
        });
    }

    pub fn mark_seq(&self, seq: i64) {
        let mut current = self.latest_seq.load(Ordering::Acquire);
        loop {
            if seq <= current {
                break;
            }

            match self.latest_seq.compare_exchange_weak(
                current,
                seq,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }

        self.pending.fetch_add(1, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Release);
    }

    pub fn mark_reset(&self) {
        self.latest_seq.store(0, Ordering::Release);
        self.pending.store(0, Ordering::Release);
        self.reset.store(true, Ordering::Release);
        self.dirty.store(true, Ordering::Release);
    }
}

/// 先持久化日志再发布游标。事件只负责唤醒，前端按批次读取已提交的 SQLite 记录。
pub async fn persist_log(
    repo: &Repository,
    app: &AppHandle,
    log: &RequestLog,
    findings: &[SecurityFinding],
    action: &str,
) -> Result<(), sqlx::Error> {
    let seq = repo.create_log(log).await?;

    if let Err(error) = repo.create_security_findings(&log.id, findings, action).await {
        tracing::warn!(log_id = %log.id, %error, "failed to persist request log security findings");
    }

    let state = app.state::<Arc<AppState>>();
    state.log_events.mark_seq(seq);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::LogEventState;
    use std::sync::atomic::Ordering;

    #[test]
    fn cursor_coalesces_writes_and_reset_is_explicit() {
        let state = LogEventState::default();

        state.mark_seq(4);
        state.mark_seq(7);
        assert_eq!(state.latest_seq.load(Ordering::Acquire), 7);
        assert_eq!(state.pending.load(Ordering::Acquire), 2);
        assert!(state.dirty.load(Ordering::Acquire));

        state.mark_reset();
        assert_eq!(state.latest_seq.load(Ordering::Acquire), 0);
        assert_eq!(state.pending.load(Ordering::Acquire), 0);
        assert!(state.reset.load(Ordering::Acquire));
    }
}
