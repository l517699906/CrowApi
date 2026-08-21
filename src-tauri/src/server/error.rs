use axum::{
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::fmt::Display;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HttpErrorDetail {
    pub message: String,
    pub r#type: String,
    pub code: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct HttpError {
    pub status: StatusCode,
    pub error: HttpErrorDetail,
}

impl HttpError {
    pub fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            status,
            error: HttpErrorDetail {
                message: message.into(),
                r#type: error_type(status).to_string(),
                code: code.into(),
                retryable,
                trace_id: None,
                details: None,
            },
        }
    }

    pub fn reported(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        source: impl Display,
    ) -> Self {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let mut error = Self::new(status, code, message, retryable);
        tracing::error!(
            trace_id = %trace_id,
            error_code = %error.error.code,
            error = %source,
            "{}",
            error.error.message,
        );
        error.error.trace_id = Some(trace_id);
        error
    }

    pub fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message, false)
    }

    pub fn unauthorized(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, code, message, false)
    }

    pub fn forbidden(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message, false)
    }

    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message, false)
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message, false)
    }

    pub fn too_many_requests(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, code, message, true)
    }

    pub fn service_unavailable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, code, message, true)
    }

    pub fn bad_gateway(
        code: impl Into<String>,
        message: impl Into<String>,
        source: impl Display,
    ) -> Self {
        Self::reported(StatusCode::BAD_GATEWAY, code, message, true, source)
    }

    pub fn not_implemented(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, code, message, false)
    }

    pub fn internal(
        code: impl Into<String>,
        message: impl Into<String>,
        source: impl Display,
    ) -> Self {
        Self::reported(StatusCode::INTERNAL_SERVER_ERROR, code, message, true, source)
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.error.details = Some(details);
        self
    }

    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.error.trace_id = Some(trace_id.into());
        self
    }

    pub fn anthropic_type(&self) -> &'static str {
        match self.status {
            StatusCode::BAD_REQUEST => "invalid_request_error",
            StatusCode::UNAUTHORIZED => "authentication_error",
            StatusCode::FORBIDDEN => "permission_error",
            StatusCode::NOT_FOUND => "not_found_error",
            StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
            _ => "api_error",
        }
    }
}

fn error_type(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "invalid_request_error",
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::FORBIDDEN => "permission_error",
        StatusCode::NOT_FOUND => "not_found_error",
        StatusCode::CONFLICT => "conflict_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        status if status.is_server_error() => "server_error",
        _ => "api_error",
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let trace_id = self.error.trace_id.clone();
        let mut response = (
            self.status,
            Json(serde_json::json!({ "error": self.error })),
        )
            .into_response();
        if let Some(trace_id) = trace_id {
            if let Ok(value) = HeaderValue::from_str(&trace_id) {
                response.headers_mut().insert("x-crowapi-trace-id", value);
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::HttpError;
    use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};

    #[tokio::test]
    async fn serializes_openai_compatible_error_envelope() {
        let response = HttpError::bad_request("MODEL_REQUIRED", "model is required")
            .with_details(serde_json::json!({ "field": "model" }))
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: serde_json::Value = serde_json::from_slice(&bytes)
            .expect("parse response body");
        assert_eq!(body["error"]["code"], "MODEL_REQUIRED");
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["details"]["field"], "model");
    }

    #[tokio::test]
    async fn reported_error_exposes_trace_but_not_internal_source() {
        let response = HttpError::internal(
            "DATABASE_ERROR",
            "读取失败",
            "sqlite path contained a secret",
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.headers().contains_key("x-crowapi-trace-id"));

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
        assert!(body.contains("DATABASE_ERROR"));
        assert!(!body.contains("sqlite path contained a secret"));
    }

    #[tokio::test]
    async fn gateway_and_capability_errors_have_stable_statuses() {
        let response = HttpError::bad_gateway(
            "UPSTREAM_REQUEST_FAILED",
            "上游请求失败",
            "provider response contained a secret",
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(response.headers().contains_key("x-crowapi-trace-id"));

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
        assert!(body.contains("UPSTREAM_REQUEST_FAILED"));
        assert!(!body.contains("provider response contained a secret"));

        assert_eq!(
            HttpError::service_unavailable("CHANNEL_UNAVAILABLE", "渠道不可用")
                .into_response()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE,
        );
        assert_eq!(
            HttpError::not_implemented("NOT_IMPLEMENTED", "接口暂未实现")
                .into_response()
                .status(),
            StatusCode::NOT_IMPLEMENTED,
        );
    }
}
