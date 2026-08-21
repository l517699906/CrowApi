use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

pub(crate) type SessionSender = mpsc::UnboundedSender<String>;

const MAX_SSE_SESSIONS: usize = 256;
pub(crate) const SSE_SESSION_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
struct SessionEntry {
    sender: SessionSender,
    principal_id: String,
    created_at: Instant,
}

fn sse_sessions() -> &'static Arc<RwLock<HashMap<String, SessionEntry>>> {
    static SESSIONS: std::sync::OnceLock<Arc<RwLock<HashMap<String, SessionEntry>>>> =
        std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

pub(crate) async fn register_sse_session(
    session_id: String,
    sender: SessionSender,
    principal_id: String,
) -> Result<(), ()> {
    let mut sessions = sse_sessions().write().await;
    let now = Instant::now();
    sessions.retain(|_, entry| {
        !entry.sender.is_closed() && now.duration_since(entry.created_at) < SSE_SESSION_TTL
    });
    if sessions.len() >= MAX_SSE_SESSIONS {
        return Err(());
    }
    sessions.insert(
        session_id,
        SessionEntry {
            sender,
            principal_id,
            created_at: now,
        },
    );
    Ok(())
}

pub(crate) async fn remove_sse_session(session_id: &str) {
    sse_sessions().write().await.remove(session_id);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionAccessError {
    NotFound,
    PrincipalMismatch,
}

pub(crate) async fn session_sender_for_principal(
    session_id: &str,
    principal_id: &str,
) -> Result<SessionSender, SessionAccessError> {
    let entry = sse_sessions()
        .read()
        .await
        .get(session_id)
        .cloned()
        .ok_or(SessionAccessError::NotFound)?;
    if entry.principal_id != principal_id {
        return Err(SessionAccessError::PrincipalMismatch);
    }
    Ok(entry.sender)
}

pub(crate) struct SessionGuard(pub(crate) String);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let session_id = self.0.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                remove_sse_session(&session_id).await;
            });
        }
    }
}
