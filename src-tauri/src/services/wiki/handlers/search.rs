use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::services::wiki::repository::WikiRepository;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_top_k() -> usize { 10 }

// ── Search & Ask ──

pub async fn search(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Query(params): Query<SearchQuery>,
) -> Response {
    let repo = WikiRepository::new(shared.state.db.pool.clone());
    match repo.search_pages_page(&id, &params.q, params.offset, params.top_k).await {
        Ok(page) => Json(serde_json::json!({
            "data": page.results,
            "query": page.query,
            "pagination": {
                "total": page.total,
                "offset": page.offset,
                "limit": page.limit,
            },
        })).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_SEARCH_FAILED",
            "搜索 Wiki 失败",
            error,
        ).into_response(),
    }
}

