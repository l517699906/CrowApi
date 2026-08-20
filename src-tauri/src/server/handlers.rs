use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{Json, IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::StreamExt;
use super::router::SharedState;
use super::error::HttpError;
use crate::core::access::{api_key_is_expired, parse_scope, scope_allows};
use crate::core::proxy;
use crate::db::models::{ApiKey, Channel};
use crate::db::repository::Repository;
use crate::adaptor::{get_adaptor, prepare_channel_request, resolve_model_mapping, ProxyRequest};
use crate::core::dispatcher::Dispatcher;
use crate::security;
use crate::protocol;

async fn persist_request_log(
    repo: &Repository,
    app: &tauri::AppHandle,
    log: &crate::db::models::RequestLog,
    scan: &security::SecurityScanResult,
) {
    if let Err(error) = crate::core::log_events::persist_log(
        repo,
        app,
        log,
        &scan.findings,
        scan.action.as_str(),
    ).await {
        tracing::warn!(log_id = %log.id, %error, "failed to persist request log");
    }
}

fn quota_limit_reached(limit: i64, used: i64) -> bool {
    limit > 0 && used >= limit
}

fn validate_api_key_scopes(
    key: &ApiKey,
) -> Result<(Vec<String>, Vec<String>), HttpError> {
    match api_key_is_expired(key.expires_at.as_deref()) {
        Ok(true) => return Err(HttpError::unauthorized("API_KEY_EXPIRED", "API key has expired")),
        Ok(false) => {}
        Err(error) => {
            return Err(HttpError::internal(
                "API_KEY_CONFIGURATION_INVALID",
                "Invalid API key configuration",
                error,
            ));
        }
    }

    let allowed_models = parse_scope(&key.allowed_models).map_err(|error| {
        HttpError::internal(
            "API_KEY_CONFIGURATION_INVALID",
            "Invalid API key configuration",
            error,
        )
    })?;
    let allowed_channels = parse_scope(&key.allowed_channels).map_err(|error| {
        HttpError::internal(
            "API_KEY_CONFIGURATION_INVALID",
            "Invalid API key configuration",
            error,
        )
    })?;
    Ok((allowed_models, allowed_channels))
}

fn validate_api_key_access(
    key: &ApiKey,
    model: &str,
) -> Result<Vec<String>, HttpError> {
    if model.trim().is_empty() {
        return Err(HttpError::bad_request("MODEL_REQUIRED", "model is required"));
    }

    let (allowed_models, allowed_channels) = validate_api_key_scopes(key)?;
    if !scope_allows(&allowed_models, model, "全部模型") {
        return Err(HttpError::forbidden(
            "MODEL_NOT_ALLOWED",
            format!("API key is not allowed to use model: {}", model),
        )
        .with_details(serde_json::json!({ "model": model })));
    }

    Ok(allowed_channels)
}

async fn quota_exceeded(
    repo: &Repository,
    app: &tauri::AppHandle,
    key: &ApiKey,
) -> Result<bool, sqlx::Error> {
    if quota_limit_reached(key.quota_limit, key.quota_used) {
        return Ok(true);
    }

    let total_quota = crate::config::load_total_quota(app);
    if total_quota == 0 {
        return Ok(false);
    }

    let total_used = repo.get_total_quota_used().await?;
    Ok(quota_limit_reached(total_quota, total_used))
}

fn anthropic_error_response(
    status: StatusCode,
    error_type: &str,
    message: impl Into<String>,
) -> Response {
    (
        status,
        Json(serde_json::json!({
            "type": "error",
            "error": {"type": error_type, "message": message.into()}
        })),
    )
        .into_response()
}

fn anthropic_http_error_response(error: HttpError) -> Response {
    let error_type = error.anthropic_type();
    let trace_id = error.error.trace_id.clone();
    let mut response = anthropic_error_response(error.status, error_type, error.error.message);
    if let Some(trace_id) = trace_id {
        if let Ok(value) = axum::http::HeaderValue::from_str(&trace_id) {
            response.headers_mut().insert("x-crowapi-trace-id", value);
        }
    }
    response
}

fn upstream_error_response(code: u16, source: impl std::fmt::Display) -> Response {
    let status = StatusCode::from_u16(code)
        .ok()
        .filter(|status| status.is_client_error() || status.is_server_error())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let retryable = status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error();
    HttpError::reported(
        status,
        "UPSTREAM_REQUEST_FAILED",
        "上游请求失败",
        retryable,
        source,
    )
    .with_details(serde_json::json!({ "upstream_status": code }))
    .into_response()
}

#[allow(clippy::too_many_arguments)]
async fn persist_anthropic_aux_log(
    repo: &Repository,
    app: &tauri::AppHandle,
    key: &ApiKey,
    channel: Option<&Channel>,
    model: &str,
    upstream_model: Option<&str>,
    request_body: &str,
    scan: &security::SecurityScanResult,
    status_code: u16,
    duration_ms: i64,
    error_message: Option<String>,
    is_retry: bool,
    trace_id: Option<String>,
) {
    let log = crate::db::models::RequestLog {
        id: crate::utils::id::new_id(),
        seq: None,
        api_key_id: Some(key.id.clone()),
        api_key_name: Some(key.name.clone()),
        channel_id: channel.map(|value| value.id.clone()),
        channel_name: channel.map(|value| value.name.clone()),
        model: model.to_string(),
        upstream_model: upstream_model.map(str::to_string),
        mode: "anthropic_count_tokens".to_string(),
        status_code: i64::from(status_code),
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        duration_ms,
        error_message,
        is_stream: 0,
        is_retry: i64::from(is_retry),
        created_at: crate::utils::time::now_iso(),
        request_body: Some(request_body.to_string()),
        response_choices: None,
        risk_level: scan.risk_level.as_str().to_string(),
        risk_score: i64::from(scan.risk_score),
        risk_summary: Some(scan.summary.clone()),
        security_action: scan.action.as_str().to_string(),
        sanitized: i64::from(scan.sanitized),
        blocked_reason: scan.blocked_reason.clone(),
        trace_id,
    };
    persist_request_log(repo, app, &log, scan).await;
}

pub async fn handle_chat_completions(
    State(shared): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    handle_chat_completions_for_mode(shared, headers, body, "chat").await
}

async fn handle_chat_completions_for_mode(
    shared: SharedState,
    headers: HeaderMap,
    body: axum::body::Bytes,
    request_mode: &'static str,
) -> Response {
    let body_str = String::from_utf8_lossy(&body);
    let json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(j) => j,
        Err(error) => {
            return HttpError::bad_request("INVALID_JSON", format!("Invalid JSON: {}", error))
                .into_response();
        }
    };

    let is_stream = json.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let model = json.get("model").and_then(|value| value.as_str()).unwrap_or("");

    let auth_header = headers.get("authorization").and_then(|h| h.to_str().ok()).unwrap_or("");
    let api_key = auth_header.strip_prefix("Bearer ").unwrap_or("").trim();

    if api_key.is_empty() {
        return HttpError::unauthorized("MISSING_API_KEY", "Missing API key").into_response();
    }

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));
    let key_record = match repo.get_api_key_by_key(api_key).await {
        Ok(k) => k,
        Err(_) => return HttpError::unauthorized("INVALID_API_KEY", "Invalid API key").into_response(),
    };
    let allowed_channels = match validate_api_key_access(&key_record, model) {
        Ok(scope) => scope,
        Err(error) => return error.into_response(),
    };

    let is_quota_exceeded = match quota_exceeded(&repo, &shared.app, &key_record).await {
        Ok(exceeded) => exceeded,
        Err(error) => {
            return HttpError::internal(
                "QUOTA_CHECK_FAILED",
                "Failed to check quota",
                error,
            )
            .into_response();
        }
    };
    if is_quota_exceeded {
        return HttpError::too_many_requests("QUOTA_EXCEEDED", "Quota exceeded").into_response();
    }

    // Extract Crow-Trace-Id from request headers
    let trace_id = headers.get("Crow-Trace-Id").and_then(|h| h.to_str().ok()).map(|s| s.to_string());

    let request_body_str = security::redact_request_body_for_log(&json);

    if is_stream {
        handle_stream(
            shared,
            json,
            key_record.id,
            key_record.name,
            request_body_str,
            trace_id,
            request_mode,
            allowed_channels,
        )
        .await
    } else {
        match proxy::handle_request(&repo, &shared.app, &key_record.id, &key_record.name, json, false, request_mode, Some(request_body_str), trace_id, Some(&allowed_channels)).await {
            Ok(result) => (StatusCode::OK, Json(result.body)).into_response(),
            Err((code, message)) => upstream_error_response(code, message),
        }
    }
}

/// Parse token usage from an SSE chunk's data line.
/// Looks for `usage` field in the JSON payload of `data: {...}` lines.
fn parse_usage_from_chunk(text: &str) -> Option<(i64, i64, i64)> {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let data_str = trimmed.trim_start_matches("data:").trim();
        if data_str == "[DONE]" || data_str.is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data_str) {
            if let Some(usage) = json.get("usage") {
                let prompt = usage.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                let completion = usage.get("completion_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                let total = usage.get("total_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                if total > 0 || prompt > 0 || completion > 0 {
                    return Some((prompt, completion, total));
                }
            }
        }
    }
    None
}

async fn handle_stream(
    shared: SharedState,
    json: serde_json::Value,
    api_key_id: String,
    api_key_name: String,
    request_body: String,
    trace_id: Option<String>,
    request_mode: &'static str,
    allowed_channels: Vec<String>,
) -> Response {
    let model = json.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let security_settings = security::get_security_settings(&shared.app);
    let security_result = security::scan_request(&json, &security_settings);

    // Real redaction: if redact mode is active, sanitize the request body before forwarding
    let (forward_json, was_redacted) = if matches!(security_result.action, security::SecurityAction::Redact) || security_settings.redact_secrets {
        security::redact_request_body(&json, &security_settings)
    } else {
        (json.clone(), false)
    };
    let mut security_result = security_result;
    if was_redacted {
        security_result.sanitized = true;
    }

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));

    if matches!(security_result.action, security::SecurityAction::Block) {
        let log = crate::db::models::RequestLog {
            response_choices: None,
            id: crate::utils::id::new_id(),
            seq: None,
            api_key_id: Some(api_key_id),
            api_key_name: Some(api_key_name),
            channel_id: None,
            channel_name: None,
            model: model.clone(),
            upstream_model: None,
            mode: request_mode.to_string(),
            status_code: 451,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            duration_ms: 0,
            error_message: security_result.blocked_reason.clone(),
            is_stream: 1,
            is_retry: 0,
            created_at: crate::utils::time::now_iso(),
            request_body: Some(request_body),
            risk_level: security_result.risk_level.as_str().to_string(),
            risk_score: security_result.risk_score as i64,
            risk_summary: Some(security_result.summary.clone()),
            security_action: security_result.action.as_str().to_string(),
            sanitized: if security_result.sanitized { 1 } else { 0 },
            blocked_reason: security_result.blocked_reason.clone(),
            trace_id: trace_id.clone(),
        };
        persist_request_log(&repo, &shared.app, &log, &security_result).await;
        return HttpError::new(
            StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
            "SECURITY_BLOCKED",
            security_result.summary,
            false,
        ).into_response();
    }
    let channels = match repo.get_enabled_channels().await {
        Ok(c) => c,
        Err(error) => return HttpError::reported(
            StatusCode::SERVICE_UNAVAILABLE,
            "CHANNEL_LIST_FAILED",
            "读取可用渠道失败",
            true,
            error,
        ).into_response(),
    };

    let model_channels = Dispatcher::select_channels(&channels, &model);
    if model_channels.is_empty() {
        return HttpError::service_unavailable(
            "CHANNEL_UNAVAILABLE",
            "当前模型没有可用渠道",
        ).into_response();
    }
    let selected_channels: Vec<_> = model_channels
        .into_iter()
        .filter(|channel| scope_allows(&allowed_channels, &channel.id, "全部渠道"))
        .collect();
    if selected_channels.is_empty() {
        return HttpError::forbidden(
            "CHANNEL_NOT_ALLOWED",
            "API Key 无权使用当前模型对应的渠道",
        ).into_response();
    }

    let request = ProxyRequest {
        model: model.clone(),
        body: forward_json.clone(),
        stream: true,
    };

    let (retry_enabled, retry_times) = proxy::get_retry_settings(&shared.app);
    let max_attempts = if retry_enabled {
        (retry_times.max(0) as usize + 1).min(selected_channels.len())
    } else {
        1
    };

    let mut last_error = None;

    for (attempt, channel) in selected_channels.into_iter().take(max_attempts).enumerate() {
        let config = Dispatcher::channel_to_config(&channel);
        let adaptor = get_adaptor(&channel.channel_type);
        let (channel_request, channel_config, upstream_model) = prepare_channel_request(&request, &config);

        match adaptor.forward_stream(&channel_request, &channel_config).await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let body_str = resp.text().await.unwrap_or_default();
                    last_error = Some(format!("{}: {}", channel.name, body_str));
                    continue;
                }

                let start = std::time::Instant::now();
                let channel_id = channel.id.clone();
                let channel_name = channel.name.clone();
                let repo_clone = repo.clone();
                let app_for_log = shared.app.clone();
                let api_key_id_clone = api_key_id.clone();
                let api_key_name_clone = api_key_name.clone();
                let model_clone = model.clone();
                let upstream_model_clone = upstream_model.clone();
                let request_body_clone = request_body.clone();
                let security_result_clone = security_result.clone();
                let trace_id_clone = trace_id.clone();
                let request_mode_clone = request_mode.to_string();
                let is_retry = if attempt > 0 { 1 } else { 0 };

                // ── Raw byte passthrough with usage parsing ───────────────
                // Forward upstream SSE bytes directly as the response body.
                // While passing through, scan data lines for `usage` to record
                // token consumption in the log.
                let upstream_stream = resp.bytes_stream();

                let passthrough_stream = async_stream::stream! {
                    tokio::pin!(upstream_stream);

                    // Accumulate token usage and response content from SSE chunks
                    let mut usage_prompt: i64 = 0;
                    let mut usage_completion: i64 = 0;
                    let mut usage_total: i64 = 0;
                    let mut had_error = false;
                    let mut accumulated_content = String::new();
                    let mut accumulated_reasoning = String::new();
                    let mut response_role: Option<String> = None;
                    let mut finish_reason: Option<String> = None;
                    // Accumulate tool_calls by index (streaming chunks may contain partial tool_calls)
                    let mut tool_calls_map: std::collections::BTreeMap<i64, serde_json::Value> = std::collections::BTreeMap::new();

                    while let Some(chunk_result) = upstream_stream.next().await {
                        match chunk_result {
                            Ok(bytes) => {
                                // Try to parse usage and content from this chunk
                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                    if let Some((p, c, t)) = parse_usage_from_chunk(text) {
                                        usage_prompt = p;
                                        usage_completion = c;
                                        usage_total = t;
                                    }
                                    // Accumulate delta content from SSE chunks
                                    for line in text.lines() {
                                        let trimmed = line.trim();
                                        if !trimmed.starts_with("data:") {
                                            continue;
                                        }
                                        let data_str = trimmed.trim_start_matches("data:").trim();
                                        if data_str == "[DONE]" || data_str.is_empty() {
                                            continue;
                                        }
                                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data_str) {
                                            if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                                                if let Some(choice) = choices.first() {
                                                    if let Some(delta) = choice.get("delta") {
                                                        // Accumulate regular content
                                                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                                            accumulated_content.push_str(content);
                                                        }
                                                        // Accumulate reasoning/thinking content (DeepSeek R1, OpenAI o1/o3, etc.)
                                                        if let Some(reasoning) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                                                            accumulated_reasoning.push_str(reasoning);
                                                        }
                                                        if response_role.is_none() {
                                                            if let Some(role) = delta.get("role").and_then(|r| r.as_str()) {
                                                                response_role = Some(role.to_string());
                                                            }
                                                        }
                                                        // Accumulate tool_calls by index
                                                        if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                                            for tc in tcs {
                                                                let idx = tc.get("index").and_then(|i| i.as_i64()).unwrap_or(0);
                                                                let entry = tool_calls_map.entry(idx).or_insert_with(|| {
                                                                    serde_json::json!({
                                                                        "id": "",
                                                                        "type": "function",
                                                                        "function": {
                                                                            "name": "",
                                                                            "arguments": ""
                                                                        }
                                                                    })
                                                                });
                                                                if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                                                    if !id.is_empty() {
                                                                        entry["id"] = serde_json::json!(id);
                                                                    }
                                                                }
                                                                if let Some(t) = tc.get("type").and_then(|v| v.as_str()) {
                                                                    if !t.is_empty() {
                                                                        entry["type"] = serde_json::json!(t);
                                                                    }
                                                                }
                                                                if let Some(func) = tc.get("function") {
                                                                    if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                                                        if !name.is_empty() {
                                                                            entry["function"]["name"] = serde_json::json!(name);
                                                                        }
                                                                    }
                                                                    if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                                                        let existing = entry["function"]["arguments"].as_str().unwrap_or("");
                                                                        entry["function"]["arguments"] = serde_json::json!(format!("{}{}", existing, args));
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    if finish_reason.is_none() {
                                                        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                                                            if !reason.is_empty() && reason != "null" {
                                                                finish_reason = Some(reason.to_string());
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                yield Ok::<_, std::io::Error>(bytes);
                            }
                            Err(e) => {
                                had_error = true;
                                let err_chunk = format!(
                                    "data: {{\"error\":{{\"message\":\"Stream connection interrupted: {}\",\"type\":\"server_error\"}}}}\n\n",
                                    e
                                );
                                yield Ok::<_, std::io::Error>(err_chunk.into_bytes().into());
                                yield Ok::<_, std::io::Error>(b"data: [DONE]\n\n".to_vec().into());
                                break;
                            }
                        }
                    }

                    // Build response_choices from accumulated streaming content
                    let has_content = !accumulated_content.is_empty() || !accumulated_reasoning.is_empty() || !tool_calls_map.is_empty();
                    let response_choices = if has_content {
                        let mut message = serde_json::json!({
                            "role": response_role.unwrap_or_else(|| "assistant".to_string()),
                        });
                        // Only include content if there is any
                        if !accumulated_content.is_empty() {
                            message["content"] = serde_json::json!(accumulated_content);
                        }
                        // Include reasoning_content if present
                        if !accumulated_reasoning.is_empty() {
                            message["reasoning_content"] = serde_json::json!(accumulated_reasoning);
                        }
                        // Include tool_calls if present
                        if !tool_calls_map.is_empty() {
                            let tcs: Vec<serde_json::Value> = tool_calls_map.into_values().collect();
                            message["tool_calls"] = serde_json::json!(tcs);
                        }
                        Some(serde_json::to_string(&vec![serde_json::json!({
                            "index": 0,
                            "message": message,
                            "finish_reason": finish_reason,
                        })]).unwrap_or_default())
                    } else {
                        None
                    };

                    // Log after stream completes
                    let quota_to_add = usage_total;
                    let key_id_for_quota = api_key_id_clone.clone();
                    let log = crate::db::models::RequestLog {
                        id: crate::utils::id::new_id(),
                        seq: None,
                        api_key_id: Some(api_key_id_clone),
                        api_key_name: Some(api_key_name_clone),
                        channel_id: Some(channel_id),
                        channel_name: Some(channel_name),
                        model: model_clone.clone(),
                        upstream_model: Some(upstream_model_clone),
                        mode: request_mode_clone,
                        status_code: if had_error { 502 } else { 200 },
                        prompt_tokens: usage_prompt,
                        completion_tokens: usage_completion,
                        total_tokens: usage_total,
                        duration_ms: start.elapsed().as_millis() as i64,
                        error_message: if had_error { Some("Stream interrupted".to_string()) } else { None },
                        is_stream: 1,
                        is_retry,
                        created_at: crate::utils::time::now_iso(),
                        request_body: Some(request_body_clone),
                        response_choices,
                        risk_level: security_result_clone.risk_level.as_str().to_string(),
                        risk_score: security_result_clone.risk_score as i64,
                        risk_summary: Some(security_result_clone.summary.clone()),
                        security_action: security_result_clone.action.as_str().to_string(),
                        sanitized: if security_result_clone.sanitized { 1 } else { 0 },
                        blocked_reason: security_result_clone.blocked_reason.clone(),
                        trace_id: trace_id_clone,
                    };
                    persist_request_log(&repo_clone, &app_for_log, &log, &security_result_clone).await;

                    // Increment quota if we got token counts
                    if quota_to_add > 0 {
                        if let Err(e) = repo_clone.increment_quota(&key_id_for_quota, quota_to_add).await { eprintln!("[WARN] increment_quota failed: {}", e); }
                    }
                };

                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive")
                    .body(Body::from_stream(passthrough_stream))
                    .unwrap();
            }
            Err(e) => {
                let error_message = e.to_string();
                let log = crate::db::models::RequestLog {
                    id: crate::utils::id::new_id(),
                    seq: None,
                    api_key_id: Some(api_key_id.clone()),
                    api_key_name: Some(api_key_name.clone()),
                    channel_id: Some(channel.id.clone()),
                    channel_name: Some(channel.name.clone()),
                    model: model.clone(),
                    upstream_model: Some(upstream_model.clone()),
                    mode: request_mode.to_string(),
                    status_code: 502,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    duration_ms: 0,
                    error_message: Some(error_message.clone()),
                    is_stream: 1,
                    is_retry: if attempt > 0 { 1 } else { 0 },
                    created_at: crate::utils::time::now_iso(),
                    request_body: Some(request_body.clone()),
                    response_choices: None,
                    risk_level: security_result.risk_level.as_str().to_string(),
                    risk_score: security_result.risk_score as i64,
                    risk_summary: Some(security_result.summary.clone()),
                    security_action: security_result.action.as_str().to_string(),
                    sanitized: if security_result.sanitized { 1 } else { 0 },
                    blocked_reason: security_result.blocked_reason.clone(),
                    trace_id: trace_id.clone(),
                };
                persist_request_log(&repo, &shared.app, &log, &security_result).await;
                last_error = Some(format!("{}: {}", channel.name, error_message));
            }
        }
    }

    HttpError::bad_gateway(
        "UPSTREAM_STREAM_FAILED",
        "所有流式渠道请求均失败",
        last_error.unwrap_or_else(|| "unknown upstream error".to_string()),
    )
    .with_details(serde_json::json!({
        "model": model,
        "attempts": max_attempts,
    }))
    .into_response()
}

// ─── Anthropic Messages API: POST /v1/messages ─────────────────────────────
// Accepts Anthropic-format requests and proxies to upstream channels.
// For Claude-type channels: forward natively (Anthropic format).
// For other channels: convert Anthropic → OpenAI → upstream → OpenAI → Anthropic.

pub async fn handle_messages(
    State(shared): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let body_str = String::from_utf8_lossy(&body);
    let json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(j) => j,
        Err(_) => return anthropic_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "请求体不是有效的 JSON",
        ),
    };

    let is_stream = json.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let model = json.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();

    // Extract API key from x-api-key header or Authorization Bearer
    let api_key = match protocol::extract_api_key(&headers) {
        Some(k) => k,
        None => return anthropic_error_response(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "缺少 API Key",
        ),
    };

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));
    let key_record = match repo.get_api_key_by_key(&api_key).await {
        Ok(k) => k,
        Err(_) => return anthropic_error_response(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "API Key 无效",
        ),
    };
    let allowed_channels = match validate_api_key_access(&key_record, &model) {
        Ok(scope) => scope,
        Err(error) => {
            return anthropic_http_error_response(error);
        }
    };

    let is_quota_exceeded = match quota_exceeded(&repo, &shared.app, &key_record).await {
        Ok(exceeded) => exceeded,
        Err(error) => {
            return anthropic_http_error_response(HttpError::internal(
                "QUOTA_CHECK_FAILED",
                "检查配额失败",
                error,
            ));
        }
    };
    if is_quota_exceeded {
        return anthropic_http_error_response(HttpError::too_many_requests(
            "QUOTA_EXCEEDED",
            "配额已用尽",
        ));
    }

    let trace_id = headers.get("Crow-Trace-Id").and_then(|h| h.to_str().ok()).map(|s| s.to_string());
    let request_body_str = security::redact_request_body_for_log(&json);

    // Convert Anthropic request to OpenAI format for internal proxy
    let openai_body = protocol::anthropic_to_openai(&json);

    if is_stream {
        handle_messages_stream(shared, openai_body, model, key_record.id, key_record.name, request_body_str, trace_id, allowed_channels).await
    } else {
        match proxy::handle_request(&repo, &shared.app, &key_record.id, &key_record.name, openai_body, false, "anthropic", Some(request_body_str), trace_id, Some(&allowed_channels)).await {
            Ok(result) => {
                // Convert OpenAI response back to Anthropic format
                let anthropic_resp = protocol::openai_to_anthropic(&result.body, &result.channel.channel_type);
                (StatusCode::OK, Json(anthropic_resp)).into_response()
            }
            Err((code, message)) => {
                let status = StatusCode::from_u16(code)
                    .ok()
                    .filter(|status| status.is_client_error() || status.is_server_error())
                    .unwrap_or(StatusCode::BAD_GATEWAY);
                anthropic_http_error_response(HttpError::reported(
                    status,
                    "UPSTREAM_REQUEST_FAILED",
                    "上游请求失败",
                    status == StatusCode::REQUEST_TIMEOUT
                        || status == StatusCode::TOO_MANY_REQUESTS
                        || status.is_server_error(),
                    message,
                ))
            }
        }
    }
}

/// Forward Anthropic's exact token-count request only to native Claude channels.
pub async fn handle_messages_count_tokens(
    State(shared): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return anthropic_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                format!("Invalid JSON: {error}"),
            );
        }
    };
    let model = match json.get("model").and_then(serde_json::Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.to_string(),
        _ => {
            return anthropic_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "model is required",
            );
        }
    };
    let api_key = match protocol::extract_api_key(&headers) {
        Some(value) => value,
        None => {
            return anthropic_error_response(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "Missing API key",
            );
        }
    };

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));
    let key_record = match repo.get_api_key_by_key(&api_key).await {
        Ok(value) => value,
        Err(_) => {
            return anthropic_error_response(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "Invalid API key",
            );
        }
    };
    let allowed_channels = match validate_api_key_access(&key_record, &model) {
        Ok(value) => value,
        Err(error) => {
            return anthropic_error_response(
                error.status,
                error.anthropic_type(),
                error.error.message,
            );
        }
    };
    match quota_exceeded(&repo, &shared.app, &key_record).await {
        Ok(true) => {
            return anthropic_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limit_error",
                "Quota exceeded",
            );
        }
        Ok(false) => {}
        Err(error) => {
            tracing::error!(%error, "count_tokens quota check failed");
            return anthropic_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "Failed to check quota",
            );
        }
    }

    let security_settings = security::get_security_settings(&shared.app);
    let mut security_result = security::scan_request(&json, &security_settings);
    let (forward_json, was_redacted) = if matches!(
        security_result.action,
        security::SecurityAction::Redact
    ) || security_settings.redact_secrets
    {
        security::redact_request_body(&json, &security_settings)
    } else {
        (json.clone(), false)
    };
    if was_redacted {
        security_result.sanitized = true;
    }
    let request_body = security::redact_request_body_for_log(&json);
    let trace_id = headers
        .get("Crow-Trace-Id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if matches!(security_result.action, security::SecurityAction::Block) {
        persist_anthropic_aux_log(
            &repo,
            &shared.app,
            &key_record,
            None,
            &model,
            None,
            &request_body,
            &security_result,
            451,
            0,
            security_result.blocked_reason.clone(),
            false,
            trace_id,
        )
        .await;
        return anthropic_error_response(
            StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
            "api_error",
            security_result.summary,
        );
    }

    let channels = match repo.get_enabled_channels().await {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "failed to load count_tokens channels");
            return anthropic_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "api_error",
                "No channels available",
            );
        }
    };
    let native_channels: Vec<_> = Dispatcher::select_channels(&channels, &model)
        .into_iter()
        .filter(|channel| channel.channel_type.eq_ignore_ascii_case("claude"))
        .collect();
    if native_channels.is_empty() {
        persist_anthropic_aux_log(
            &repo,
            &shared.app,
            &key_record,
            None,
            &model,
            None,
            &request_body,
            &security_result,
            501,
            0,
            Some("Exact Anthropic count_tokens requires a native Claude channel".to_string()),
            false,
            trace_id,
        )
        .await;
        return anthropic_error_response(
            StatusCode::NOT_IMPLEMENTED,
            "api_error",
            "Exact Anthropic count_tokens requires a native Claude channel",
        );
    }
    let selected_channels: Vec<_> = native_channels
        .into_iter()
        .filter(|channel| scope_allows(&allowed_channels, &channel.id, "全部渠道"))
        .collect();
    if selected_channels.is_empty() {
        return anthropic_error_response(
            StatusCode::FORBIDDEN,
            "permission_error",
            "API key is not allowed to use a native Claude channel for this model",
        );
    }

    let (retry_enabled, retry_times) = proxy::get_retry_settings(&shared.app);
    let max_attempts = if retry_enabled {
        (retry_times.max(0) as usize + 1).min(selected_channels.len())
    } else {
        1
    };
    let client = reqwest::Client::new();
    let mut last_error = None;
    let mut last_response = None;

    for (attempt, channel) in selected_channels.into_iter().take(max_attempts).enumerate() {
        let config = Dispatcher::channel_to_config(&channel);
        let upstream_model = resolve_model_mapping(&model, &config.model_mapping);
        let mut upstream_body = forward_json.clone();
        upstream_body["model"] = serde_json::Value::String(upstream_model.clone());
        let url = format!(
            "{}/messages/count_tokens",
            config.base_url.trim_end_matches('/')
        );
        let started = std::time::Instant::now();
        let mut request = client
            .post(url)
            .header("x-api-key", &config.api_key)
            .header(
                "anthropic-version",
                headers
                    .get("anthropic-version")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("2023-06-01"),
            )
            .header(header::CONTENT_TYPE, "application/json")
            .timeout(std::time::Duration::from_secs(config.timeout_secs.max(1)))
            .json(&upstream_body);
        if let Some(value) = headers.get("anthropic-beta") {
            request = request.header("anthropic-beta", value.clone());
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let status_code = status.as_u16();
                let response_body = response.bytes().await.unwrap_or_default();
                let error_message = if status.is_success() {
                    None
                } else {
                    Some(String::from_utf8_lossy(&response_body).chars().take(500).collect())
                };
                persist_anthropic_aux_log(
                    &repo,
                    &shared.app,
                    &key_record,
                    Some(&channel),
                    &model,
                    Some(&upstream_model),
                    &request_body,
                    &security_result,
                    status_code,
                    started.elapsed().as_millis() as i64,
                    error_message.clone(),
                    attempt > 0,
                    trace_id.clone(),
                )
                .await;

                let retryable = status == StatusCode::REQUEST_TIMEOUT
                    || status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error();
                if status.is_success() || !retryable {
                    return Response::builder()
                        .status(status)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(response_body))
                        .unwrap();
                }
                last_error = error_message;
                last_response = Some((status, response_body));
            }
            Err(error) => {
                let message = error.to_string();
                persist_anthropic_aux_log(
                    &repo,
                    &shared.app,
                    &key_record,
                    Some(&channel),
                    &model,
                    Some(&upstream_model),
                    &request_body,
                    &security_result,
                    502,
                    started.elapsed().as_millis() as i64,
                    Some(message.clone()),
                    attempt > 0,
                    trace_id.clone(),
                )
                .await;
                last_error = Some(message);
            }
        }
    }

    if let Some((status, response_body)) = last_response {
        return Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(response_body))
            .unwrap();
    }

    anthropic_error_response(
        StatusCode::BAD_GATEWAY,
        "api_error",
        format!(
            "All native Claude channels failed: {}",
            last_error.unwrap_or_else(|| "unknown upstream error".to_string())
        ),
    )
}

/// Stream handler for Anthropic Messages API.
/// Converts OpenAI SSE stream to Anthropic SSE events.
async fn handle_messages_stream(
    shared: SharedState,
    openai_body: serde_json::Value,
    model: String,
    api_key_id: String,
    api_key_name: String,
    request_body: String,
    trace_id: Option<String>,
    allowed_channels: Vec<String>,
) -> Response {
    let security_settings = security::get_security_settings(&shared.app);
    let security_result = security::scan_request(&openai_body, &security_settings);

    let (forward_json, was_redacted) = if matches!(security_result.action, security::SecurityAction::Redact) || security_settings.redact_secrets {
        security::redact_request_body(&openai_body, &security_settings)
    } else {
        (openai_body.clone(), false)
    };
    let mut security_result = security_result;
    if was_redacted {
        security_result.sanitized = true;
    }

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));

    if matches!(security_result.action, security::SecurityAction::Block) {
        let log = crate::db::models::RequestLog {
            response_choices: None,
            id: crate::utils::id::new_id(),
            seq: None,
            api_key_id: Some(api_key_id.clone()),
            api_key_name: Some(api_key_name.clone()),
            channel_id: None,
            channel_name: None,
            model: model.clone(),
            upstream_model: None,
            mode: "anthropic".to_string(),
            status_code: 451,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            duration_ms: 0,
            error_message: security_result.blocked_reason.clone(),
            is_stream: 1,
            is_retry: 0,
            created_at: crate::utils::time::now_iso(),
            request_body: Some(request_body),
            risk_level: security_result.risk_level.as_str().to_string(),
            risk_score: security_result.risk_score as i64,
            risk_summary: Some(security_result.summary.clone()),
            security_action: security_result.action.as_str().to_string(),
            sanitized: if security_result.sanitized { 1 } else { 0 },
            blocked_reason: security_result.blocked_reason.clone(),
            trace_id: trace_id.clone(),
        };
        persist_request_log(&repo, &shared.app, &log, &security_result).await;
        let err_body = serde_json::json!({"type": "error", "error": {"type": "api_error", "message": security_result.summary}});
        return (StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS, Json(err_body)).into_response();
    }

    let channels = match repo.get_enabled_channels().await {
        Ok(c) => c,
        Err(error) => return anthropic_http_error_response(HttpError::reported(
            StatusCode::SERVICE_UNAVAILABLE,
            "CHANNEL_LIST_FAILED",
            "读取可用渠道失败",
            true,
            error,
        )),
    };

    let model_channels = Dispatcher::select_channels(&channels, &model);
    if model_channels.is_empty() {
        return anthropic_http_error_response(HttpError::service_unavailable(
            "CHANNEL_UNAVAILABLE",
            "当前模型没有可用渠道",
        ));
    }
    let selected_channels: Vec<_> = model_channels
        .into_iter()
        .filter(|channel| scope_allows(&allowed_channels, &channel.id, "全部渠道"))
        .collect();
    if selected_channels.is_empty() {
        return anthropic_http_error_response(HttpError::forbidden(
            "CHANNEL_NOT_ALLOWED",
            "API Key 无权使用当前模型对应的渠道",
        ));
    }

    let request = ProxyRequest { model: model.clone(), body: forward_json.clone(), stream: true };
    let (retry_enabled, retry_times) = proxy::get_retry_settings(&shared.app);
    let max_attempts = if retry_enabled {
        (retry_times.max(0) as usize + 1).min(selected_channels.len())
    } else { 1 };

    let mut last_error = None;

    for (attempt, channel) in selected_channels.into_iter().take(max_attempts).enumerate() {
        let config = Dispatcher::channel_to_config(&channel);
        let adaptor = get_adaptor(&channel.channel_type);
        let (channel_request, channel_config, upstream_model) = prepare_channel_request(&request, &config);

        match adaptor.forward_stream(&channel_request, &channel_config).await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let body_str = resp.text().await.unwrap_or_default();
                    last_error = Some(format!("{}: {}", channel.name, body_str));
                    continue;
                }

                let start = std::time::Instant::now();
                let channel_id = channel.id.clone();
                let channel_name = channel.name.clone();
                let repo_clone = repo.clone();
                let app_for_log = shared.app.clone();
                let api_key_id_clone = api_key_id.clone();
                let api_key_name_clone = api_key_name.clone();
                let model_clone = model.clone();
                let upstream_model_clone = upstream_model.clone();
                let request_body_clone = request_body.clone();
                let security_result_clone = security_result.clone();
                let trace_id_clone = trace_id.clone();
                let is_retry = if attempt > 0 { 1 } else { 0 };

                let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
                let upstream_stream = resp.bytes_stream();

                let passthrough_stream = async_stream::stream! {
                    tokio::pin!(upstream_stream);

                    let mut state = crate::protocol::anthropic::AnthropicStreamState::default();
                    let mut usage_prompt: i64 = 0;
                    let mut usage_completion: i64 = 0;
                    let mut usage_total: i64 = 0;
                    let mut had_error = false;
                    let mut accumulated_content = String::new();

                    while let Some(chunk_result) = upstream_stream.next().await {
                        match chunk_result {
                            Ok(bytes) => {
                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                    if let Some((p, c, t)) = crate::protocol::anthropic::parse_usage_from_sse_chunk(text) {
                                        usage_prompt = p;
                                        usage_completion = c;
                                        usage_total = t;
                                    }
                                    // Accumulate content for logging
                                    for line in text.lines() {
                                        let trimmed = line.trim();
                                        if !trimmed.starts_with("data:") { continue; }
                                        let data_str = trimmed.trim_start_matches("data:").trim();
                                        if data_str == "[DONE]" || data_str.is_empty() { continue; }
                                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data_str) {
                                            if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                                                if let Some(choice) = choices.first() {
                                                    if let Some(delta) = choice.get("delta") {
                                                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                                            accumulated_content.push_str(content);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // Convert OpenAI SSE → Anthropic SSE events
                                    let events = crate::protocol::anthropic::convert_openai_sse_to_anthropic(
                                        text, &model_clone, &message_id, &mut state
                                    );
                                    for event in events {
                                        yield Ok::<_, std::io::Error>(event.into_bytes().into());
                                    }
                                } else {
                                    yield Ok::<_, std::io::Error>(bytes);
                                }
                            }
                            Err(e) => {
                                had_error = true;
                                let err_event = format!(
                                    "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"api_error\",\"message\":\"Stream interrupted: {}\"}}}}\n\n",
                                    e
                                );
                                yield Ok::<_, std::io::Error>(err_event.into_bytes().into());
                                break;
                            }
                        }
                    }

                    // Build response_choices for logging
                    let response_choices = if !accumulated_content.is_empty() {
                        Some(serde_json::to_string(&vec![serde_json::json!({
                            "index": 0,
                            "message": {"role": "assistant", "content": accumulated_content},
                            "finish_reason": "stop",
                        })]).unwrap_or_default())
                    } else { None };

                    let log = crate::db::models::RequestLog {
                        id: crate::utils::id::new_id(),
                        seq: None,
                        api_key_id: Some(api_key_id_clone.clone()),
                        api_key_name: Some(api_key_name_clone.clone()),
                        channel_id: Some(channel_id),
                        channel_name: Some(channel_name),
                        model: model_clone.clone(),
                        upstream_model: Some(upstream_model_clone),
                        mode: "anthropic".to_string(),
                        status_code: if had_error { 502 } else { 200 },
                        prompt_tokens: usage_prompt,
                        completion_tokens: usage_completion,
                        total_tokens: usage_total,
                        duration_ms: start.elapsed().as_millis() as i64,
                        error_message: if had_error { Some("Stream interrupted".to_string()) } else { None },
                        is_stream: 1,
                        is_retry,
                        created_at: crate::utils::time::now_iso(),
                        request_body: Some(request_body_clone),
                        response_choices,
                        risk_level: security_result_clone.risk_level.as_str().to_string(),
                        risk_score: security_result_clone.risk_score as i64,
                        risk_summary: Some(security_result_clone.summary.clone()),
                        security_action: security_result_clone.action.as_str().to_string(),
                        sanitized: if security_result_clone.sanitized { 1 } else { 0 },
                        blocked_reason: security_result_clone.blocked_reason.clone(),
                        trace_id: trace_id_clone,
                    };
                    persist_request_log(&repo_clone, &app_for_log, &log, &security_result_clone).await;
                    if usage_total > 0 {
                        if let Err(e) = repo_clone.increment_quota(&api_key_id_clone, usage_total).await { eprintln!("[WARN] increment_quota failed: {}", e); }
                    }
                };

                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive")
                    .body(Body::from_stream(passthrough_stream))
                    .unwrap();
            }
            Err(e) => {
                let error_message = e.to_string();
                let log = crate::db::models::RequestLog {
                    id: crate::utils::id::new_id(),
                    seq: None,
                    api_key_id: Some(api_key_id.clone()),
                    api_key_name: Some(api_key_name.clone()),
                    channel_id: Some(channel.id.clone()),
                    channel_name: Some(channel.name.clone()),
                    model: model.clone(),
                    upstream_model: Some(upstream_model.clone()),
                    mode: "anthropic".to_string(),
                    status_code: 502,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    duration_ms: 0,
                    error_message: Some(error_message.clone()),
                    is_stream: 1,
                    is_retry: if attempt > 0 { 1 } else { 0 },
                    created_at: crate::utils::time::now_iso(),
                    request_body: Some(request_body.clone()),
                    response_choices: None,
                    risk_level: security_result.risk_level.as_str().to_string(),
                    risk_score: security_result.risk_score as i64,
                    risk_summary: Some(security_result.summary.clone()),
                    security_action: security_result.action.as_str().to_string(),
                    sanitized: if security_result.sanitized { 1 } else { 0 },
                    blocked_reason: security_result.blocked_reason.clone(),
                    trace_id: trace_id.clone(),
                };
                persist_request_log(&repo, &shared.app, &log, &security_result).await;
                last_error = Some(format!("{}: {}", channel.name, error_message));
            }
        }
    }

    anthropic_http_error_response(
        HttpError::bad_gateway(
            "UPSTREAM_STREAM_FAILED",
            "所有流式渠道请求均失败",
            last_error.unwrap_or_else(|| "unknown upstream error".to_string()),
        )
        .with_details(serde_json::json!({
            "model": model,
            "attempts": max_attempts,
        })),
    )
}

// ─── OpenAI Responses API: POST /v1/responses ────────────────────────────────
// Accepts Responses API format and proxies to upstream channels via Chat Completions.
// Converts: Responses input → OpenAI messages → upstream → OpenAI response → Responses output.

pub async fn handle_responses(
    State(shared): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let body_str = String::from_utf8_lossy(&body);
    let json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(j) => j,
        Err(_) => return HttpError::bad_request(
            "INVALID_JSON",
            "请求体不是有效的 JSON",
        ).into_response(),
    };

    let is_stream = json.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let model = json.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();

    let api_key = match protocol::extract_api_key(&headers) {
        Some(k) => k,
        None => return HttpError::unauthorized(
            "MISSING_API_KEY",
            "缺少 API Key",
        ).into_response(),
    };

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));
    let key_record = match repo.get_api_key_by_key(&api_key).await {
        Ok(k) => k,
        Err(_) => return HttpError::unauthorized(
            "INVALID_API_KEY",
            "API Key 无效",
        ).into_response(),
    };
    let allowed_channels = match validate_api_key_access(&key_record, &model) {
        Ok(scope) => scope,
        Err(error) => return error.into_response(),
    };

    let is_quota_exceeded = match quota_exceeded(&repo, &shared.app, &key_record).await {
        Ok(exceeded) => exceeded,
        Err(error) => {
            return HttpError::internal(
                "QUOTA_CHECK_FAILED",
                "检查配额失败",
                error,
            ).into_response();
        }
    };
    if is_quota_exceeded {
        return HttpError::too_many_requests(
            "QUOTA_EXCEEDED",
            "配额已用尽",
        ).into_response();
    }

    let trace_id = headers.get("Crow-Trace-Id").and_then(|h| h.to_str().ok()).map(|s| s.to_string());
    let request_body_str = security::redact_request_body_for_log(&json);

    // Convert Responses API request to OpenAI Chat Completions format
    let openai_body = protocol::responses_to_openai(&json);

    if is_stream {
        handle_responses_stream(shared, openai_body, model, key_record.id, key_record.name, request_body_str, trace_id, allowed_channels).await
    } else {
        match proxy::handle_request(&repo, &shared.app, &key_record.id, &key_record.name, openai_body, false, "responses", Some(request_body_str), trace_id, Some(&allowed_channels)).await {
            Ok(result) => {
                // Convert OpenAI response to Responses API format
                let responses_resp = protocol::openai_to_responses(&result.body, &model);
                (StatusCode::OK, Json(responses_resp)).into_response()
            }
            Err((code, message)) => upstream_error_response(code, message),
        }
    }
}

/// Stream handler for Responses API.
/// Converts OpenAI SSE stream to Responses API SSE events.
async fn handle_responses_stream(
    shared: SharedState,
    openai_body: serde_json::Value,
    model: String,
    api_key_id: String,
    api_key_name: String,
    request_body: String,
    trace_id: Option<String>,
    allowed_channels: Vec<String>,
) -> Response {
    let security_settings = security::get_security_settings(&shared.app);
    let security_result = security::scan_request(&openai_body, &security_settings);

    let (forward_json, was_redacted) = if matches!(security_result.action, security::SecurityAction::Redact) || security_settings.redact_secrets {
        security::redact_request_body(&openai_body, &security_settings)
    } else {
        (openai_body.clone(), false)
    };
    let mut security_result = security_result;
    if was_redacted {
        security_result.sanitized = true;
    }

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));

    if matches!(security_result.action, security::SecurityAction::Block) {
        let log = crate::db::models::RequestLog {
            response_choices: None,
            id: crate::utils::id::new_id(),
            seq: None,
            api_key_id: Some(api_key_id.clone()),
            api_key_name: Some(api_key_name.clone()),
            channel_id: None,
            channel_name: None,
            model: model.clone(),
            upstream_model: None,
            mode: "responses".to_string(),
            status_code: 451,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            duration_ms: 0,
            error_message: security_result.blocked_reason.clone(),
            is_stream: 1,
            is_retry: 0,
            created_at: crate::utils::time::now_iso(),
            request_body: Some(request_body),
            risk_level: security_result.risk_level.as_str().to_string(),
            risk_score: security_result.risk_score as i64,
            risk_summary: Some(security_result.summary.clone()),
            security_action: security_result.action.as_str().to_string(),
            sanitized: if security_result.sanitized { 1 } else { 0 },
            blocked_reason: security_result.blocked_reason.clone(),
            trace_id: trace_id.clone(),
        };
        persist_request_log(&repo, &shared.app, &log, &security_result).await;
        let err_body = serde_json::json!({"error": {"message": security_result.summary, "type": "security_blocked"}});
        return (StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS, Json(err_body)).into_response();
    }

    let channels = match repo.get_enabled_channels().await {
        Ok(c) => c,
        Err(error) => return HttpError::reported(
            StatusCode::SERVICE_UNAVAILABLE,
            "CHANNEL_LIST_FAILED",
            "读取可用渠道失败",
            true,
            error,
        ).into_response(),
    };

    let model_channels = Dispatcher::select_channels(&channels, &model);
    if model_channels.is_empty() {
        return HttpError::service_unavailable(
            "CHANNEL_UNAVAILABLE",
            "当前模型没有可用渠道",
        ).into_response();
    }
    let selected_channels: Vec<_> = model_channels
        .into_iter()
        .filter(|channel| scope_allows(&allowed_channels, &channel.id, "全部渠道"))
        .collect();
    if selected_channels.is_empty() {
        return HttpError::forbidden(
            "CHANNEL_NOT_ALLOWED",
            "API Key 无权使用当前模型对应的渠道",
        ).into_response();
    }

    let request = ProxyRequest { model: model.clone(), body: forward_json.clone(), stream: true };
    let (retry_enabled, retry_times) = proxy::get_retry_settings(&shared.app);
    let max_attempts = if retry_enabled {
        (retry_times.max(0) as usize + 1).min(selected_channels.len())
    } else { 1 };

    let mut last_error = None;

    for (attempt, channel) in selected_channels.into_iter().take(max_attempts).enumerate() {
        let config = Dispatcher::channel_to_config(&channel);
        let adaptor = get_adaptor(&channel.channel_type);
        let (channel_request, channel_config, upstream_model) = prepare_channel_request(&request, &config);

        match adaptor.forward_stream(&channel_request, &channel_config).await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let body_str = resp.text().await.unwrap_or_default();
                    last_error = Some(format!("{}: {}", channel.name, body_str));
                    continue;
                }

                let start = std::time::Instant::now();
                let channel_id = channel.id.clone();
                let channel_name = channel.name.clone();
                let repo_clone = repo.clone();
                let app_for_log = shared.app.clone();
                let api_key_id_clone = api_key_id.clone();
                let api_key_name_clone = api_key_name.clone();
                let model_clone = model.clone();
                let upstream_model_clone = upstream_model.clone();
                let request_body_clone = request_body.clone();
                let security_result_clone = security_result.clone();
                let trace_id_clone = trace_id.clone();
                let is_retry = if attempt > 0 { 1 } else { 0 };

                let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
                let upstream_stream = resp.bytes_stream();

                let passthrough_stream = async_stream::stream! {
                    tokio::pin!(upstream_stream);

                    // Emit response.created event
                    let created = crate::protocol::responses::create_response_created_event(&model_clone, &response_id);
                    yield Ok::<_, std::io::Error>(created.into_bytes().into());

                    let mut usage_prompt: i64 = 0;
                    let mut usage_completion: i64 = 0;
                    let mut usage_total: i64 = 0;
                    let mut had_error = false;
                    let mut accumulated_content = String::new();

                    while let Some(chunk_result) = upstream_stream.next().await {
                        match chunk_result {
                            Ok(bytes) => {
                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                    if let Some((p, c, t)) = crate::protocol::responses::parse_usage_from_sse_chunk(text) {
                                        usage_prompt = p;
                                        usage_completion = c;
                                        usage_total = t;
                                    }
                                    // Accumulate content for logging
                                    for line in text.lines() {
                                        let trimmed = line.trim();
                                        if !trimmed.starts_with("data:") { continue; }
                                        let data_str = trimmed.trim_start_matches("data:").trim();
                                        if data_str == "[DONE]" || data_str.is_empty() { continue; }
                                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data_str) {
                                            if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                                                if let Some(choice) = choices.first() {
                                                    if let Some(delta) = choice.get("delta") {
                                                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                                            accumulated_content.push_str(content);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // Convert OpenAI SSE → Responses SSE events
                                    let events = crate::protocol::responses::convert_openai_sse_to_responses(
                                        text, &model_clone, &response_id
                                    );
                                    for event in events {
                                        yield Ok::<_, std::io::Error>(event.into_bytes().into());
                                    }
                                } else {
                                    yield Ok::<_, std::io::Error>(bytes);
                                }
                            }
                            Err(e) => {
                                had_error = true;
                                let err_event = format!(
                                    "event: response.failed\ndata: {{\"type\":\"response.failed\",\"response_id\":\"{}\",\"error\":{{\"message\":\"Stream interrupted: {}\"}}}}\n\n",
                                    response_id, e
                                );
                                yield Ok::<_, std::io::Error>(err_event.into_bytes().into());
                                break;
                            }
                        }
                    }

                    // Build response_choices for logging
                    let response_choices = if !accumulated_content.is_empty() {
                        Some(serde_json::to_string(&vec![serde_json::json!({
                            "index": 0,
                            "message": {"role": "assistant", "content": accumulated_content},
                            "finish_reason": "stop",
                        })]).unwrap_or_default())
                    } else { None };

                    let log = crate::db::models::RequestLog {
                        id: crate::utils::id::new_id(),
                        seq: None,
                        api_key_id: Some(api_key_id_clone.clone()),
                        api_key_name: Some(api_key_name_clone.clone()),
                        channel_id: Some(channel_id),
                        channel_name: Some(channel_name),
                        model: model_clone.clone(),
                        upstream_model: Some(upstream_model_clone),
                        mode: "responses".to_string(),
                        status_code: if had_error { 502 } else { 200 },
                        prompt_tokens: usage_prompt,
                        completion_tokens: usage_completion,
                        total_tokens: usage_total,
                        duration_ms: start.elapsed().as_millis() as i64,
                        error_message: if had_error { Some("Stream interrupted".to_string()) } else { None },
                        is_stream: 1,
                        is_retry,
                        created_at: crate::utils::time::now_iso(),
                        request_body: Some(request_body_clone),
                        response_choices,
                        risk_level: security_result_clone.risk_level.as_str().to_string(),
                        risk_score: security_result_clone.risk_score as i64,
                        risk_summary: Some(security_result_clone.summary.clone()),
                        security_action: security_result_clone.action.as_str().to_string(),
                        sanitized: if security_result_clone.sanitized { 1 } else { 0 },
                        blocked_reason: security_result_clone.blocked_reason.clone(),
                        trace_id: trace_id_clone,
                    };
                    persist_request_log(&repo_clone, &app_for_log, &log, &security_result_clone).await;
                    if usage_total > 0 {
                        if let Err(e) = repo_clone.increment_quota(&api_key_id_clone, usage_total).await { eprintln!("[WARN] increment_quota failed: {}", e); }
                    }
                };

                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive")
                    .body(Body::from_stream(passthrough_stream))
                    .unwrap();
            }
            Err(e) => {
                let error_message = e.to_string();
                let log = crate::db::models::RequestLog {
                    id: crate::utils::id::new_id(),
                    seq: None,
                    api_key_id: Some(api_key_id.clone()),
                    api_key_name: Some(api_key_name.clone()),
                    channel_id: Some(channel.id.clone()),
                    channel_name: Some(channel.name.clone()),
                    model: model.clone(),
                    upstream_model: Some(upstream_model.clone()),
                    mode: "responses".to_string(),
                    status_code: 502,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    duration_ms: 0,
                    error_message: Some(error_message.clone()),
                    is_stream: 1,
                    is_retry: if attempt > 0 { 1 } else { 0 },
                    created_at: crate::utils::time::now_iso(),
                    request_body: Some(request_body.clone()),
                    response_choices: None,
                    risk_level: security_result.risk_level.as_str().to_string(),
                    risk_score: security_result.risk_score as i64,
                    risk_summary: Some(security_result.summary.clone()),
                    security_action: security_result.action.as_str().to_string(),
                    sanitized: if security_result.sanitized { 1 } else { 0 },
                    blocked_reason: security_result.blocked_reason.clone(),
                    trace_id: trace_id.clone(),
                };
                persist_request_log(&repo, &shared.app, &log, &security_result).await;
                last_error = Some(format!("{}: {}", channel.name, error_message));
            }
        }
    }

    HttpError::bad_gateway(
        "UPSTREAM_STREAM_FAILED",
        "所有流式渠道请求均失败",
        last_error.unwrap_or_else(|| "unknown upstream error".to_string()),
    )
    .with_details(serde_json::json!({
        "model": model,
        "attempts": max_attempts,
    }))
    .into_response()
}

fn legacy_prompt_to_text(prompt: &serde_json::Value) -> Result<String, String> {
    match prompt {
        serde_json::Value::String(value) => Ok(value.clone()),
        serde_json::Value::Array(values) => {
            if values.len() != 1 {
                return Err("批量 prompt 暂不支持，请每次提交一个 prompt".to_string());
            }
            values[0]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "prompt 数组目前只支持单个字符串元素".to_string())
        }
        _ => Err("prompt 必须是字符串或字符串数组".to_string()),
    }
}

fn message_content_text(content: &serde_json::Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

fn convert_chat_response_to_completion(
    mut body: serde_json::Value,
    model: &str,
    prompt_text: &str,
    echo: bool,
) -> serde_json::Value {
    let Some(object) = body.as_object_mut() else {
        return body;
    };

    object.insert("object".to_string(), serde_json::json!("text_completion"));
    object.insert("model".to_string(), serde_json::json!(model));
    if let Some(choices) = object.get_mut("choices").and_then(|value| value.as_array_mut()) {
        for (index, choice) in choices.iter_mut().enumerate() {
            let text = choice
                .get("message")
                .and_then(|message| message.get("content"))
                .map(message_content_text)
                .unwrap_or_default();
            let text = if echo && index == 0 {
                format!("{}{}", prompt_text, text)
            } else {
                text
            };
            let finish_reason = choice.get("finish_reason").cloned().unwrap_or(serde_json::Value::Null);
            let choice_index = choice.get("index").cloned().unwrap_or_else(|| serde_json::json!(index));
            *choice = serde_json::json!({
                "text": text,
                "index": choice_index,
                "logprobs": null,
                "finish_reason": finish_reason,
            });
        }
    }
    body
}

fn convert_chat_sse_event(
    event: &str,
    model: &str,
    prompt_text: &str,
    echo: bool,
    echo_written: &mut bool,
    done: &mut bool,
) -> Option<String> {
    let data = event
        .lines()
        .find_map(|line| line.trim().strip_prefix("data:").map(str::trim))?;
    if data == "[DONE]" {
        *done = true;
        return Some("data: [DONE]\n\n".to_string());
    }

    let payload = serde_json::from_str::<serde_json::Value>(data).ok()?;
    let choice = payload.get("choices")?.as_array()?.first()?;
    let delta = choice.get("delta").cloned().unwrap_or_default();
    let mut text = delta
        .get("content")
        .map(message_content_text)
        .unwrap_or_default();
    if echo && !*echo_written {
        text = format!("{}{}", prompt_text, text);
        *echo_written = true;
    }
    let finish_reason = choice.get("finish_reason").cloned().unwrap_or(serde_json::Value::Null);
    let index = choice.get("index").cloned().unwrap_or_else(|| serde_json::json!(0));
    let id = payload
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or("cmpl-compat");
    let created = payload.get("created").cloned().unwrap_or_else(|| serde_json::json!(0));
    let response = serde_json::json!({
        "id": id,
        "object": "text_completion",
        "created": created,
        "model": model,
        "choices": [{
            "text": text,
            "index": index,
            "logprobs": null,
            "finish_reason": finish_reason,
        }]
    });
    Some(format!("data: {}\n\n", response))
}

/// Compatibility adapter for the legacy OpenAI text-completions API.
/// The request is translated to chat messages and the response is translated back,
/// so all authentication, quota, routing, security, and retry behavior stays shared.
pub async fn handle_completions(
    State(shared): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let body_str = String::from_utf8_lossy(&body);
    let mut json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(value) => value,
        Err(_) => return HttpError::bad_request(
            "INVALID_JSON",
            "请求体不是有效的 JSON",
        ).into_response(),
    };

    let Some(object) = json.as_object_mut() else {
        return HttpError::bad_request(
            "REQUEST_OBJECT_REQUIRED",
            "请求体必须是 JSON 对象",
        ).into_response();
    };
    let Some(model) = object.get("model").and_then(|value| value.as_str()).map(str::to_string) else {
        return HttpError::bad_request("MODEL_REQUIRED", "model 为必填项").into_response();
    };
    let Some(prompt) = object.get("prompt") else {
        return HttpError::bad_request("PROMPT_REQUIRED", "prompt 为必填项").into_response();
    };
    let prompt_text = match legacy_prompt_to_text(prompt) {
        Ok(value) => value,
        Err(error) => return HttpError::bad_request(
            "PROMPT_INVALID",
            error,
        ).into_response(),
    };
    let stream = object.get("stream").and_then(|value| value.as_bool()).unwrap_or(false);
    let echo = object.get("echo").and_then(|value| value.as_bool()).unwrap_or(false);
    object.remove("prompt");
    object.insert(
        "messages".to_string(),
        serde_json::json!([{ "role": "user", "content": prompt_text }]),
    );
    object.insert("stream".to_string(), serde_json::json!(stream));

    let translated = match serde_json::to_vec(&json) {
        Ok(value) => Bytes::from(value),
        Err(error) => return HttpError::internal(
            "REQUEST_TRANSLATION_FAILED",
            "转换兼容请求失败",
            error,
        ).into_response(),
    };
    let chat_response = handle_chat_completions_for_mode(shared, headers, translated, "completion").await;
    if !chat_response.status().is_success() {
        return chat_response;
    }

    if !stream {
        let body = match to_bytes(chat_response.into_body(), 16 * 1024 * 1024).await {
            Ok(value) => value,
            Err(error) => return HttpError::bad_gateway(
                "UPSTREAM_RESPONSE_READ_FAILED",
                "读取上游响应失败",
                error,
            ).into_response(),
        };
        let chat_json = match serde_json::from_slice::<serde_json::Value>(&body) {
            Ok(value) => value,
            Err(error) => return HttpError::bad_gateway(
                "UPSTREAM_RESPONSE_INVALID",
                "上游响应格式无效",
                error,
            ).into_response(),
        };
        return (
            StatusCode::OK,
            Json(convert_chat_response_to_completion(
                chat_json,
                &model,
                &prompt_text,
                echo,
            )),
        )
            .into_response();
    }

    let mut upstream_stream = chat_response.into_body().into_data_stream();
    let model_for_stream = model.clone();
    let prompt_for_stream = prompt_text.clone();
    let stream_body = async_stream::stream! {
        let mut buffer = String::new();
        let mut echo_written = false;
        let mut done = false;

        while let Some(chunk) = upstream_stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(position) = buffer.find("\n\n") {
                        let event = buffer[..position + 2].to_string();
                        buffer.drain(..position + 2);
                        if let Some(converted) = convert_chat_sse_event(
                            &event,
                            &model_for_stream,
                            &prompt_for_stream,
                            echo,
                            &mut echo_written,
                            &mut done,
                        ) {
                            yield Ok::<Bytes, std::io::Error>(Bytes::from(converted));
                        }
                    }
                }
                Err(error) => {
                    let message = serde_json::json!({
                        "error": { "message": format!("Stream connection interrupted: {}", error), "type": "server_error" }
                    });
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(format!("data: {}\n\n", message)));
                    break;
                }
            }
        }

        if !buffer.trim().is_empty() && !done {
            if let Some(converted) = convert_chat_sse_event(
                &buffer,
                &model_for_stream,
                &prompt_for_stream,
                echo,
                &mut echo_written,
                &mut done,
            ) {
                yield Ok::<Bytes, std::io::Error>(Bytes::from(converted));
            }
        }
        if !done {
            yield Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: [DONE]\n\n"));
        }
    };

    let mut response = Response::new(Body::from_stream(stream_body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        "text/event-stream".parse().expect("valid content type"),
    );
    response
}

fn supports_openai_embeddings(channel_type: &str) -> bool {
    matches!(channel_type, "openai" | "custom")
}

fn build_embedding_forward_body(
    request: &serde_json::Value,
    upstream_model: &str,
) -> serde_json::Value {
    let mut body = request.clone();
    body["model"] = serde_json::Value::String(upstream_model.to_string());
    body
}

pub async fn handle_embeddings(
    State(shared): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let body_str = String::from_utf8_lossy(&body);
    let json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(j) => j,
        Err(_) => return HttpError::bad_request(
            "INVALID_JSON",
            "请求体不是有效的 JSON",
        ).into_response(),
    };

    let api_key = match protocol::extract_api_key(&headers) {
        Some(k) => k,
        None => return HttpError::unauthorized(
            "MISSING_API_KEY",
            "缺少 API Key",
        ).into_response(),
    };

    let repo = std::sync::Arc::new(Repository::new(shared.state.db.pool.clone()));
    let key_record = match repo.get_api_key_by_key(&api_key).await {
        Ok(k) => k,
        Err(_) => return HttpError::unauthorized(
            "INVALID_API_KEY",
            "API Key 无效",
        ).into_response(),
    };

    let model = json.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
    if model.is_empty() || json.get("input").is_none() {
        return HttpError::bad_request(
            "EMBEDDING_INPUT_REQUIRED",
            "model 和 input 为必填项",
        ).into_response();
    }
    let allowed_channels = match validate_api_key_access(&key_record, &model) {
        Ok(scope) => scope,
        Err(error) => return error.into_response(),
    };

    let is_quota_exceeded = match quota_exceeded(&repo, &shared.app, &key_record).await {
        Ok(exceeded) => exceeded,
        Err(error) => {
            return HttpError::internal(
                "QUOTA_CHECK_FAILED",
                "检查配额失败",
                error,
            ).into_response();
        }
    };
    if is_quota_exceeded {
        return HttpError::too_many_requests(
            "QUOTA_EXCEEDED",
            "配额已用尽",
        ).into_response();
    }

    let trace_id = headers.get("Crow-Trace-Id").and_then(|h| h.to_str().ok()).map(|s| s.to_string());
    let request_body_str = security::redact_request_body_for_log(&json);

    // Security scan
    let security_settings = security::get_security_settings(&shared.app);
    let mut security_result = security::scan_request(&json, &security_settings);
    let (forward_json, was_redacted) = if matches!(security_result.action, security::SecurityAction::Redact)
        || security_settings.redact_secrets
    {
        security::redact_request_body(&json, &security_settings)
    } else {
        (json.clone(), false)
    };
    if was_redacted {
        security_result.sanitized = true;
    }

    if matches!(security_result.action, security::SecurityAction::Block) {
        let log = crate::db::models::RequestLog {
            response_choices: None,
            id: crate::utils::id::new_id(),
            seq: None,
            api_key_id: Some(key_record.id.clone()),
            api_key_name: Some(key_record.name.clone()),
            channel_id: None,
            channel_name: None,
            model: model.clone(),
            upstream_model: None,
            mode: "embedding".to_string(),
            status_code: 451,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            duration_ms: 0,
            error_message: security_result.blocked_reason.clone(),
            is_stream: 0,
            is_retry: 0,
            created_at: crate::utils::time::now_iso(),
            request_body: Some(request_body_str),
            risk_level: security_result.risk_level.as_str().to_string(),
            risk_score: security_result.risk_score as i64,
            risk_summary: Some(security_result.summary.clone()),
            security_action: security_result.action.as_str().to_string(),
            sanitized: if security_result.sanitized { 1 } else { 0 },
            blocked_reason: security_result.blocked_reason.clone(),
            trace_id: trace_id.clone(),
        };
        persist_request_log(&repo, &shared.app, &log, &security_result).await;
        return (StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS, Json(serde_json::json!({
            "error": {"message": security_result.summary, "type": "security_blocked"}
        }))).into_response();
    }

    // Select channels
    let channels = match repo.get_enabled_channels().await {
        Ok(c) => c,
        Err(error) => return HttpError::reported(
            StatusCode::SERVICE_UNAVAILABLE,
            "CHANNEL_LIST_FAILED",
            "读取可用渠道失败",
            true,
            error,
        ).into_response(),
    };

    let model_channels: Vec<_> = Dispatcher::select_channels(&channels, &model)
        .into_iter()
        .filter(|channel| supports_openai_embeddings(&channel.channel_type))
        .collect();
    if model_channels.is_empty() {
        return HttpError::service_unavailable(
            "EMBEDDING_CHANNEL_UNAVAILABLE",
            "当前模型没有兼容的向量渠道",
        ).into_response();
    }
    let selected_channels: Vec<_> = model_channels
        .into_iter()
        .filter(|channel| scope_allows(&allowed_channels, &channel.id, "全部渠道"))
        .collect();
    if selected_channels.is_empty() {
        return HttpError::forbidden(
            "CHANNEL_NOT_ALLOWED",
            "API Key 无权使用当前模型对应的渠道",
        ).into_response();
    }

    let (retry_enabled, retry_times) = proxy::get_retry_settings(&shared.app);
    let max_attempts = if retry_enabled {
        (retry_times.max(0) as usize + 1).min(selected_channels.len())
    } else { 1 };

    let mut last_error = None;
    let start = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    for (attempt, channel) in selected_channels.into_iter().take(max_attempts).enumerate() {
        let config = Dispatcher::channel_to_config(&channel);
        let upstream_model = resolve_model_mapping(&model, &config.model_mapping);

        // Build upstream embedding request — send directly to /embeddings
        // (adaptor.forward() hard-codes /chat/completions which doesn't work for embeddings)
        let base_url = config.base_url.trim_end_matches('/');
        let embed_url = format!("{}/embeddings", base_url);
        let embed_body = build_embedding_forward_body(&forward_json, &upstream_model);

        let result = client
            .post(&embed_url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .json(&embed_body)
            .timeout(std::time::Duration::from_secs(channel.timeout_secs.max(1) as u64))
            .send()
            .await;

        match result {
            Ok(resp) => {
                let status = resp.status();
                let resp_body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);

                if !status.is_success() {
                    let error_message = format!("HTTP {}: {}", status, serde_json::to_string(&resp_body).unwrap_or_default().chars().take(300).collect::<String>());
                    let log = crate::db::models::RequestLog {
                        id: crate::utils::id::new_id(),
                        seq: None,
                        api_key_id: Some(key_record.id.clone()),
                        api_key_name: Some(key_record.name.clone()),
                        channel_id: Some(channel.id.clone()),
                        channel_name: Some(channel.name.clone()),
                        model: model.clone(),
                        upstream_model: Some(upstream_model.clone()),
                        mode: "embedding".to_string(),
                        status_code: status.as_u16() as i64,
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                        duration_ms: start.elapsed().as_millis() as i64,
                        error_message: Some(error_message.clone()),
                        is_stream: 0,
                        is_retry: if attempt > 0 { 1 } else { 0 },
                        created_at: crate::utils::time::now_iso(),
                        request_body: Some(request_body_str.clone()),
                        response_choices: None,
                        risk_level: security_result.risk_level.as_str().to_string(),
                        risk_score: security_result.risk_score as i64,
                        risk_summary: Some(security_result.summary.clone()),
                        security_action: security_result.action.as_str().to_string(),
                        sanitized: if security_result.sanitized { 1 } else { 0 },
                        blocked_reason: security_result.blocked_reason.clone(),
                        trace_id: trace_id.clone(),
                    };
                    persist_request_log(&repo, &shared.app, &log, &security_result).await;
                    last_error = Some(error_message);
                    continue;
                }

                // Extract usage from response
                let usage_total = resp_body.get("usage").and_then(|u| u.get("total_tokens")).and_then(|t| t.as_u64()).unwrap_or(0) as i64;
                let usage_prompt = resp_body.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(|t| t.as_u64()).unwrap_or(0) as i64;

                let log = crate::db::models::RequestLog {
                    id: crate::utils::id::new_id(),
                    seq: None,
                    api_key_id: Some(key_record.id.clone()),
                    api_key_name: Some(key_record.name.clone()),
                    channel_id: Some(channel.id.clone()),
                    channel_name: Some(channel.name.clone()),
                    model: model.clone(),
                    upstream_model: Some(upstream_model.clone()),
                    mode: "embedding".to_string(),
                    status_code: status.as_u16() as i64,
                    prompt_tokens: usage_prompt,
                    completion_tokens: 0,
                    total_tokens: usage_total,
                    duration_ms: start.elapsed().as_millis() as i64,
                    error_message: None,
                    is_stream: 0,
                    is_retry: if attempt > 0 { 1 } else { 0 },
                    created_at: crate::utils::time::now_iso(),
                    request_body: Some(request_body_str.clone()),
                    response_choices: None,
                    risk_level: security_result.risk_level.as_str().to_string(),
                    risk_score: security_result.risk_score as i64,
                    risk_summary: Some(security_result.summary.clone()),
                    security_action: security_result.action.as_str().to_string(),
                    sanitized: if security_result.sanitized { 1 } else { 0 },
                    blocked_reason: security_result.blocked_reason.clone(),
                    trace_id: trace_id.clone(),
                };
                persist_request_log(&repo, &shared.app, &log, &security_result).await;
                if usage_total > 0 {
                    if let Err(e) = repo.increment_quota(&key_record.id, usage_total).await { eprintln!("[WARN] increment_quota failed: {}", e); }
                }

                return (StatusCode::OK, Json(resp_body)).into_response();
            }
            Err(e) => {
                let error_message = e.to_string();
                let log = crate::db::models::RequestLog {
                    id: crate::utils::id::new_id(),
                    seq: None,
                    api_key_id: Some(key_record.id.clone()),
                    api_key_name: Some(key_record.name.clone()),
                    channel_id: Some(channel.id.clone()),
                    channel_name: Some(channel.name.clone()),
                    model: model.clone(),
                    upstream_model: Some(upstream_model.clone()),
                    mode: "embedding".to_string(),
                    status_code: 502,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    duration_ms: start.elapsed().as_millis() as i64,
                    error_message: Some(error_message.clone()),
                    is_stream: 0,
                    is_retry: if attempt > 0 { 1 } else { 0 },
                    created_at: crate::utils::time::now_iso(),
                    request_body: Some(request_body_str.clone()),
                    response_choices: None,
                    risk_level: security_result.risk_level.as_str().to_string(),
                    risk_score: security_result.risk_score as i64,
                    risk_summary: Some(security_result.summary.clone()),
                    security_action: security_result.action.as_str().to_string(),
                    sanitized: if security_result.sanitized { 1 } else { 0 },
                    blocked_reason: security_result.blocked_reason.clone(),
                    trace_id: trace_id.clone(),
                };
                persist_request_log(&repo, &shared.app, &log, &security_result).await;
                last_error = Some(error_message);
            }
        }
    }

    HttpError::bad_gateway(
        "UPSTREAM_EMBEDDING_FAILED",
        "所有向量渠道请求均失败",
        last_error.unwrap_or_else(|| "unknown upstream error".to_string()),
    )
    .with_details(serde_json::json!({
        "model": model,
        "attempts": max_attempts,
    }))
    .into_response()
}

pub async fn handle_list_models(
    State(shared): State<SharedState>,
    headers: HeaderMap,
) -> Response {
    let repo = Repository::new(shared.state.db.pool.clone());
    let api_key = match protocol::extract_api_key(&headers) {
        Some(key) => key,
        None => return HttpError::unauthorized(
            "MISSING_API_KEY",
            "缺少 API Key",
        ).into_response(),
    };
    let key_record = match repo.get_api_key_by_key(&api_key).await {
        Ok(key) => key,
        Err(_) => return HttpError::unauthorized(
            "INVALID_API_KEY",
            "API Key 无效",
        ).into_response(),
    };
    let (allowed_models, allowed_channels) = match validate_api_key_scopes(&key_record) {
        Ok(scopes) => scopes,
        Err(error) => return error.into_response(),
    };

    match repo.get_enabled_channels().await {
        Ok(channels) => {
            let mut models: Vec<serde_json::Value> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for ch in channels
                .iter()
                .filter(|channel| scope_allows(&allowed_channels, &channel.id, "全部渠道"))
            {
                let ch_models: Vec<String> = serde_json::from_str(&ch.models).unwrap_or_default();
                for m in ch_models {
                    if scope_allows(&allowed_models, &m, "全部模型") && seen.insert(m.clone()) {
                        models.push(serde_json::json!({
                            "id": m, "object": "model",
                            "created": chrono::Utc::now().timestamp(),
                            "owned_by": ch.channel_type,
                        }));
                    }
                }
                // Also expose mapped model names (mapping keys)
                let mapping: serde_json::Value =
                    serde_json::from_str(&ch.model_mapping).unwrap_or(serde_json::Value::Object(Default::default()));
                if let Some(obj) = mapping.as_object() {
                    for key in obj.keys() {
                        if scope_allows(&allowed_models, key, "全部模型") && seen.insert(key.clone()) {
                            models.push(serde_json::json!({
                                "id": key, "object": "model",
                                "created": chrono::Utc::now().timestamp(),
                                "owned_by": ch.channel_type,
                            }));
                        }
                    }
                }
            }
            Json(serde_json::json!({ "object": "list", "data": models })).into_response()
        }
        Err(error) => HttpError::internal(
            "MODEL_LIST_FAILED",
            "读取模型列表失败",
            error,
        ).into_response(),
    }
}

pub async fn handle_images(State(_shared): State<SharedState>) -> Response {
    HttpError::not_implemented(
        "IMAGES_NOT_IMPLEMENTED",
        "图像接口暂未实现",
    ).into_response()
}

pub async fn handle_audio_transcriptions(State(_shared): State<SharedState>) -> Response {
    HttpError::not_implemented(
        "AUDIO_TRANSCRIPTIONS_NOT_IMPLEMENTED",
        "音频转写接口暂未实现",
    ).into_response()
}

pub async fn handle_audio_speech(State(_shared): State<SharedState>) -> Response {
    HttpError::not_implemented(
        "AUDIO_SPEECH_NOT_IMPLEMENTED",
        "语音合成接口暂未实现",
    ).into_response()
}

pub async fn handle_health(State(shared): State<SharedState>) -> Response {
    let port = shared.state.server_port.read().await.clone();
    let running = shared.state.server_running.load(std::sync::atomic::Ordering::SeqCst);
    Json(serde_json::json!({
        "status": "ok",
        "running": running,
        "port": port,
        "url": format!("http://127.0.0.1:{}", port),
    })).into_response()
}

#[cfg(test)]
mod quota_tests {
    use super::{
        build_embedding_forward_body,
        convert_chat_response_to_completion,
        legacy_prompt_to_text,
        quota_limit_reached,
        supports_openai_embeddings,
    };

    #[test]
    fn zero_quota_is_unlimited() {
        assert!(!quota_limit_reached(0, 10_000));
    }

    #[test]
    fn positive_quota_is_reached_at_the_limit() {
        assert!(!quota_limit_reached(100, 99));
        assert!(quota_limit_reached(100, 100));
        assert!(quota_limit_reached(100, 101));
    }

    #[test]
    fn legacy_prompt_rejects_ambiguous_batches() {
        let prompt = serde_json::json!(["first", "second"]);
        assert!(legacy_prompt_to_text(&prompt).is_err());
    }

    #[test]
    fn chat_response_is_translated_to_text_completion() {
        let chat = serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "model": "upstream-model",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": " world" },
                "finish_reason": "stop"
            }]
        });
        let converted = convert_chat_response_to_completion(chat, "requested-model", "hello", true);
        assert_eq!(converted["object"], "text_completion");
        assert_eq!(converted["model"], "requested-model");
        assert_eq!(converted["choices"][0]["text"], "hello world");
        assert!(converted["choices"][0].get("message").is_none());
    }

    #[test]
    fn embeddings_are_limited_to_openai_compatible_channels() {
        assert!(supports_openai_embeddings("openai"));
        assert!(supports_openai_embeddings("custom"));
        assert!(!supports_openai_embeddings("claude"));
        assert!(!supports_openai_embeddings("gemini"));
        assert!(!supports_openai_embeddings("deepseek"));
    }

    #[test]
    fn embedding_forward_body_preserves_standard_options() {
        let request = serde_json::json!({
            "model": "public-model",
            "input": ["alpha", "beta"],
            "encoding_format": "base64",
            "dimensions": 512,
            "user": "tenant-1"
        });
        let body = build_embedding_forward_body(&request, "upstream-model");
        assert_eq!(body["model"], "upstream-model");
        assert_eq!(body["input"], serde_json::json!(["alpha", "beta"]));
        assert_eq!(body["encoding_format"], "base64");
        assert_eq!(body["dimensions"], 512);
        assert_eq!(body["user"], "tenant-1");
    }
}
