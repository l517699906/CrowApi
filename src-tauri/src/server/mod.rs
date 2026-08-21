pub mod router;
pub mod handlers;
pub mod error;
pub mod auth;

use crate::AppState;
use tauri::{AppHandle, Emitter};

pub async fn start_server(app: AppHandle, state: std::sync::Arc<AppState>) -> Result<(), anyhow::Error> {
    let settings = crate::config::load_settings(&app);
    let host = settings.server_host;
    let port = settings.server_port;

    if !settings.allow_remote_access && !crate::config::is_loopback_host(&host) {
        anyhow::bail!("remote server binding requires allow_remote_access");
    }

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    let actual_port = local_addr.port();

    // 更新共享状态（前端通过命令查询）
    *state.server_port.write().await = actual_port;
    state.server_running.store(true, std::sync::atomic::Ordering::SeqCst);

    let router = router::create_router(app.clone(), state.clone());

    // 通知前端服务器已启动（前端监听此事件更新 UI）
    app.emit(
        "server-started",
        serde_json::json!({
            "port": actual_port,
            "url": crate::config::server_url(&host, actual_port)
        }),
    )
    .ok();

    tracing::info!("CrowAPI server listening on {}", crate::config::server_url(&host, actual_port));

    // 启动 Axum 服务（阻塞直到服务器停止）
    let result = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await;

    state.server_running.store(false, std::sync::atomic::Ordering::SeqCst);
    *state.server_port.write().await = 0;

    result.map_err(Into::into)
}
