use axum::{
    extract::DefaultBodyLimit,
    http::{header, HeaderValue, Method},
    middleware,
    Router,
    routing::{get, post},
};
use std::sync::Arc;
use tauri::AppHandle;
use tower_http::cors::CorsLayer;
use crate::AppState;
use super::handlers::*;

pub fn create_router(app: AppHandle, state: Arc<AppState>) -> Router {
    let shared = SharedState { app: app.clone(), state: state.clone() };

    let settings = crate::config::load_settings(&app);
    let allowed_origins = settings
        .allowed_origins
        .iter()
        .filter_map(|origin| match origin.parse::<HeaderValue>() {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(%origin, %error, "ignoring invalid CORS origin");
                None
            }
        })
        .collect::<Vec<_>>();
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::ORIGIN,
            header::HeaderName::from_static("x-api-key"),
            header::HeaderName::from_static("anthropic-version"),
            header::HeaderName::from_static("crow-trace-id"),
            header::HeaderName::from_static("mcp-protocol-version"),
            header::HeaderName::from_static("mcp-session-id"),
            header::HeaderName::from_static("last-event-id"),
        ])
        .expose_headers([
            header::HeaderName::from_static("x-crowapi-trace-id"),
            header::HeaderName::from_static("mcp-session-id"),
        ]);

    // Service registry — merge all service routes
    let registry = crate::services::ServiceRegistry::new();
    let service_router = registry.merge_routes(state.clone());
    let task_router = crate::services::tasks::routes::create_router();

    let trusted_proxy_cidrs = crate::config::parse_trusted_proxy_cidrs(&settings.trusted_proxy_cidrs)
        .unwrap_or_default();
    let auth = super::auth::AuthLayerState::new(
        shared.clone(),
        settings.allow_remote_access,
        trusted_proxy_cidrs,
    );

    Router::new()
        // OpenAI Chat Completions
        .route("/v1/chat/completions", post(handle_chat_completions))
        // OpenAI Completions (legacy)
        .route("/v1/completions", post(handle_completions))
        // OpenAI Responses API
        .route("/v1/responses", post(handle_responses))
        // OpenAI Embeddings
        .route("/v1/embeddings", post(handle_embeddings))
        // OpenAI Models
        .route("/v1/models", get(handle_list_models))
        // OpenAI Images
        .route("/v1/images/generations", post(handle_images))
        // OpenAI Audio
        .route("/v1/audio/transcriptions", post(handle_audio_transcriptions))
        .route("/v1/audio/speech", post(handle_audio_speech))
        // Anthropic Messages API
        .route(
            "/v1/messages",
            post(handle_messages).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route(
            "/v1/messages/count_tokens",
            post(handle_messages_count_tokens).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        // Health check
        .route("/health", get(handle_health))
        .route("/health/live", get(handle_health_live))
        .route("/health/ready", get(handle_health_ready))
        .route("/api/diagnostics", get(handle_diagnostics))
        // Service routes (Knowledge Base, MCP, etc.)
        .merge(service_router)
        .merge(task_router)
        .layer(middleware::from_fn_with_state(
            auth,
            super::auth::enforce_access_policy,
        ))
        .layer(cors)
        .with_state(shared)
}

#[derive(Clone)]
pub struct SharedState {
    pub app: AppHandle,
    pub state: Arc<AppState>,
}
