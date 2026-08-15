pub mod router;
pub mod handlers;

use crate::AppState;
use tauri::{AppHandle, Emitter};

pub async fn start_server(app: AppHandle, state: std::sync::Arc<AppState>) -> Result<(), anyhow::Error> {
    let settings = crate::config::load_settings(&app);
    let host = settings.server_host;
    let port = settings.server_port;

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
            "url": format!("http://{}:{}", host, actual_port)
        }),
    )
    .ok();

    tracing::info!("WaLiAPI server listening on http://{}:{}", host, actual_port);

    // 启动 Axum 服务（阻塞直到服务器停止）
    axum::serve(listener, router).await?;

    state.server_running.store(false, std::sync::atomic::Ordering::SeqCst);

    Ok(())
}
