use crate::core::proxy;
use crate::db::repository::Repository;
use crate::server::error::HttpError;
use crate::server::router::SharedState;
use crate::services::wiki::{
    models::{WikiAnswerSource, WikiAskInput},
    project,
    repository::WikiRepository,
};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Json, Response},
};
use std::sync::Arc;

pub async fn ask(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<WikiAskInput>,
) -> Response {
    let top_k = input.top_k.unwrap_or(5);
    let model = input.model.as_deref();

    match ask_inner(&shared, &id, &input.question, top_k, model).await {
        Ok(json) => Json(json).into_response(),
        Err(error) => HttpError::internal(
            "WIKI_ASK_FAILED",
            "Wiki 问答失败",
            error,
        ).into_response(),
    }
}

/// Inner logic for Wiki Q&A — shared by HTTP handler and MCP handler.
pub async fn ask_inner(
    shared: &SharedState,
    id: &str,
    question: &str,
    top_k: usize,
    model_override: Option<&str>,
) -> Result<serde_json::Value, String> {
    let pool = &shared.state.db.pool;
    let repo = WikiRepository::new(pool.clone());
    let db_repo = Arc::new(Repository::new(pool.clone()));
    let app = shared.app.clone();

    // Search relevant pages
    let results = repo.search_pages(id, question, top_k).await?;

    // Read page contents
    let mut contexts = Vec::new();
    for r in &results {
        match project::read_page(id, &r.path).await {
            Ok(content) => {
                let snippet: String = content.chars().take(2000).collect();
                contexts.push(format!("## {} ({})\n{}", r.title, r.path, snippet));
            }
            Err(error) => {
                tracing::warn!(%error, project_id = %id, page_path = %r.path, "failed to read Wiki search result");
            }
        }
    }

    if contexts.is_empty() {
        return Ok(serde_json::json!({
            "answer": "No relevant wiki pages found for your question. Please ingest some documents first.",
            "sources": []
        }));
    }

    let context_text = contexts.join("\n\n---\n\n");

    // Get project config
    let proj = repo.get_project(id).await?;

    let chat_model = model_override
        .or(proj.chat_model.as_deref())
        .unwrap_or("gpt-4o");
    let chat_channel_id = match proj.chat_channel_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            let row: Option<(String,)> = sqlx::query_as("SELECT id FROM channels WHERE status = 1 ORDER BY priority DESC LIMIT 1")
                .fetch_optional(pool).await
                .map_err(|e| format!("DB error: {}", e))?;
            match row.map(|(id,)| id) {
                Some(id) => id,
                None => return Err("No active channel configured. Please create a channel first or set chat_channel_id in Wiki project settings.".to_string()),
            }
        }
    };

    let system_prompt = "You are a Wiki knowledge assistant. Answer questions based on the provided wiki pages. Be concise and cite source pages using [[wikilinks]] format.";
    let user_prompt = format!(
        "Based on the following wiki pages, answer the question.\n\nWiki pages:\n{}\n\nQuestion: {}\n\nAnswer:",
        context_text, question
    );

    // Save user message
    if let Err(error) = repo.add_session(id, "user", question, None, None).await {
        tracing::warn!(%error, project_id = %id, "failed to persist Wiki user message");
    }

    let chat_request = serde_json::json!({
        "model": chat_model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ],
        "stream": false,
        "temperature": 0.4
    });
    let chat_request_str: String = serde_json::to_string(&chat_request).unwrap_or_default();

    let proxy_result = proxy::handle_request(
        &db_repo,
        &app,
        &chat_channel_id,
        "Wiki Chat",
        chat_request,
        false,
        "chat",
        Some(chat_request_str),
        Some(format!("wiki-chat_{}", id)),
        None,
    ).await;

    let (answer, usage) = match proxy_result {
        Ok(result) => {
            let answer_text = result.body
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|t| t.as_str())
                .unwrap_or("Failed to generate answer.");

            let usage = result.body.get("usage").map(|u| serde_json::json!({
                "prompt_tokens": u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "completion_tokens": u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "total_tokens": u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            }));

            (answer_text.to_string(), usage)
        }
        Err((code, msg)) => {
            let err_answer = format!("LLM request failed ({}): {}", code, msg);
            (err_answer, None)
        }
    };

    let sources: Vec<WikiAnswerSource> = results.iter().map(|r| WikiAnswerSource {
        path: r.path.clone(),
        title: r.title.clone(),
        score: r.score,
        snippet: r.snippet.clone(),
    }).collect();

    // Save assistant message
    if let Err(error) = repo
        .add_session(
            id,
            "assistant",
            &answer,
            Some(&serde_json::to_string(&sources).unwrap_or_default()),
            Some(chat_model),
        )
        .await
    {
        tracing::warn!(%error, project_id = %id, "failed to persist Wiki assistant message");
    }

    Ok(serde_json::json!({
        "answer": answer,
        "sources": sources,
        "usage": usage,
    }))
}

