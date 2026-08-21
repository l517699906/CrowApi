use super::error::HttpError;
use super::router::SharedState;
use crate::core::access::{
    access_scope_allows, api_key_is_expired, parse_access_scopes, ACCESS_SCOPE_ADMIN,
    ACCESS_SCOPE_GATEWAY, ACCESS_SCOPE_MCP_READ,
};
use crate::db::models::ApiKey;
use crate::db::repository::Repository;
use crate::config::IpCidr;
    use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RATE_WINDOW: Duration = Duration::from_secs(60);
const GATEWAY_REQUESTS_PER_WINDOW: u32 = 240;
const ADMIN_REQUESTS_PER_WINDOW: u32 = 120;
const MCP_REQUESTS_PER_WINDOW: u32 = 120;
const REMOTE_AUTH_REQUESTS_PER_WINDOW: u32 = 120;

#[derive(Debug, Clone)]
pub struct AuthenticatedPrincipal {
    pub api_key: ApiKey,
    access_scopes: Arc<[String]>,
}

impl AuthenticatedPrincipal {
    pub(crate) fn new(api_key: ApiKey, access_scopes: Vec<String>) -> Self {
        Self {
            api_key,
            access_scopes: access_scopes.into(),
        }
    }

    pub fn allows(&self, required: &str) -> bool {
        access_scope_allows(&self.access_scopes, required)
    }
}

#[derive(Debug)]
enum AuthenticationFailure {
    Missing,
    Invalid,
    Expired(AuthenticatedPrincipal),
    Configuration(String),
    Repository(sqlx::Error),
}

async fn authenticate_api_key(
    pool: &sqlx::SqlitePool,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, AuthenticationFailure> {
    let raw_key = crate::protocol::extract_api_key(headers)
        .ok_or(AuthenticationFailure::Missing)?;
    let repository = Repository::new(pool.clone());
    let api_key = match repository.get_api_key_by_key(&raw_key).await {
        Ok(api_key) => api_key,
        Err(sqlx::Error::RowNotFound) => return Err(AuthenticationFailure::Invalid),
        Err(error) => return Err(AuthenticationFailure::Repository(error)),
    };
    let access_scopes = parse_access_scopes(&api_key.access_scopes)
        .map_err(AuthenticationFailure::Configuration)?;
    let principal = AuthenticatedPrincipal::new(api_key, access_scopes);
    match api_key_is_expired(principal.api_key.expires_at.as_deref()) {
        Ok(true) => return Err(AuthenticationFailure::Expired(principal)),
        Ok(false) => {}
        Err(error) => return Err(AuthenticationFailure::Configuration(error.to_string())),
    }
    Ok(principal)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AccessPolicy {
    Gateway,
    Admin,
    Mcp,
}

impl AccessPolicy {
    fn required_scope(self) -> &'static str {
        match self {
            Self::Gateway => ACCESS_SCOPE_GATEWAY,
            Self::Admin => ACCESS_SCOPE_ADMIN,
            Self::Mcp => ACCESS_SCOPE_MCP_READ,
        }
    }

    fn rate_limit(self) -> u32 {
        match self {
            Self::Gateway => GATEWAY_REQUESTS_PER_WINDOW,
            Self::Admin => ADMIN_REQUESTS_PER_WINDOW,
            Self::Mcp => MCP_REQUESTS_PER_WINDOW,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Admin => "admin",
            Self::Mcp => "mcp",
        }
    }
}

fn access_policy(path: &str) -> Option<AccessPolicy> {
    if path == "/health" || path == "/health/live" || path == "/health/ready" {
        None
    } else if path == "/v1" || path.starts_with("/v1/") {
        Some(AccessPolicy::Gateway)
    } else if path == "/mcp" || path.starts_with("/mcp/") {
        Some(AccessPolicy::Mcp)
    } else {
        // Unknown and future routes fail closed behind the administrative scope.
        Some(AccessPolicy::Admin)
    }
}

#[derive(Debug)]
struct RateBucket {
    started_at: Instant,
    requests: u32,
}

#[derive(Debug, Default)]
struct PrincipalRateLimiter {
    buckets: Mutex<HashMap<(String, AccessPolicy), RateBucket>>,
}

impl PrincipalRateLimiter {
    fn check(&self, principal_id: &str, policy: AccessPolicy, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        buckets.retain(|_, bucket| now.duration_since(bucket.started_at) < RATE_WINDOW);
        let bucket = buckets
            .entry((principal_id.to_string(), policy))
            .or_insert(RateBucket {
                started_at: now,
                requests: 0,
            });
        if bucket.requests >= policy.rate_limit() {
            return false;
        }
        bucket.requests += 1;
        true
    }
}

#[derive(Debug, Default)]
struct SourceRateLimiter {
    buckets: Mutex<HashMap<(IpAddr, AccessPolicy), RateBucket>>,
}

impl SourceRateLimiter {
    fn check(&self, source: IpAddr, policy: AccessPolicy, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        buckets.retain(|_, bucket| now.duration_since(bucket.started_at) < RATE_WINDOW);
        let bucket = buckets.entry((source, policy)).or_insert(RateBucket {
            started_at: now,
            requests: 0,
        });
        if bucket.requests >= REMOTE_AUTH_REQUESTS_PER_WINDOW {
            return false;
        }
        bucket.requests += 1;
        true
    }
}

#[derive(Clone)]
pub struct AuthLayerState {
    shared: SharedState,
    limiter: Arc<PrincipalRateLimiter>,
    source_limiter: Arc<SourceRateLimiter>,
    remote_access_enabled: bool,
    trusted_proxy_cidrs: Arc<[IpCidr]>,
}

impl AuthLayerState {
    pub fn new(
        shared: SharedState,
        remote_access_enabled: bool,
        trusted_proxy_cidrs: Vec<IpCidr>,
    ) -> Self {
        Self {
            shared,
            limiter: Arc::new(PrincipalRateLimiter::default()),
            source_limiter: Arc::new(SourceRateLimiter::default()),
            remote_access_enabled,
            trusted_proxy_cidrs: trusted_proxy_cidrs.into(),
        }
    }
}

fn peer_ip(request: &Request) -> IpAddr {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
}

fn parse_forwarded_ip(value: &str) -> Option<IpAddr> {
    let mut value = value.trim().trim_matches('"');
    if value.starts_with('[') {
        value = value.strip_prefix('[')?.split(']').next()?;
    } else if value.matches(':').count() == 1 {
        if let Some((address, port)) = value.rsplit_once(':') {
            if port.parse::<u16>().is_ok() {
                value = address;
            }
        }
    }
    value.parse::<IpAddr>().ok()
}

fn forwarded_chain(headers: &HeaderMap) -> Vec<IpAddr> {
    let x_forwarded_for = header::HeaderName::from_static("x-forwarded-for");
    if let Some(value) = headers.get(&x_forwarded_for).and_then(|value| value.to_str().ok()) {
        let parsed = value
            .split(',')
            .filter_map(parse_forwarded_ip)
            .collect::<Vec<_>>();
        if !parsed.is_empty() {
            return parsed;
        }
    }

    let Some(value) = headers
        .get(header::FORWARDED)
        .and_then(|value| value.to_str().ok())
    else {
        return Vec::new();
    };
    value
        .split(',')
        .filter_map(|entry| {
            entry.split(';').find_map(|parameter| {
                let (name, value) = parameter.trim().split_once('=')?;
                name.eq_ignore_ascii_case("for")
                    .then(|| parse_forwarded_ip(value))
                    .flatten()
            })
        })
        .collect()
}

fn resolve_source_ip(request: &Request, trusted_proxy_cidrs: &[IpCidr]) -> IpAddr {
    let peer = peer_ip(request);
    if !trusted_proxy_cidrs.iter().any(|network| network.contains(peer)) {
        return peer;
    }
    let chain = forwarded_chain(request.headers());
    if chain.is_empty() {
        return peer;
    }
    let mut addresses = chain;
    addresses.push(peer);
    addresses
        .iter()
        .rev()
        .find(|address| !trusted_proxy_cidrs.iter().any(|network| network.contains(**address)))
        .copied()
        .unwrap_or(peer)
}

struct AuditContext {
    method: String,
    path: String,
    origin: Option<String>,
}

impl AuditContext {
    fn from_request(request: &Request) -> Self {
        Self {
            method: request.method().to_string(),
            path: request.uri().path().to_string(),
            origin: request
                .headers()
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        }
    }
}

async fn audit_denied(
    shared: &SharedState,
    context: &AuditContext,
    principal: Option<&AuthenticatedPrincipal>,
    outcome: &str,
    error_code: &str,
    trace_id: &str,
) {
    let result = sqlx::query(
        "INSERT INTO auth_audit_events
         (id, api_key_id, api_key_name, method, path, origin, outcome, error_code, trace_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(principal.map(|value| value.api_key.id.as_str()))
    .bind(principal.map(|value| value.api_key.name.as_str()))
    .bind(&context.method)
    .bind(&context.path)
    .bind(&context.origin)
    .bind(outcome)
    .bind(error_code)
    .bind(trace_id)
    .bind(crate::utils::time::now_iso())
    .execute(&shared.state.db.pool)
    .await;
    match result {
        Ok(_) => {
            if let Err(error) = sqlx::query(
                "DELETE FROM auth_audit_events
                 WHERE id IN (
                    SELECT id FROM auth_audit_events
                    ORDER BY created_at DESC
                    LIMIT -1 OFFSET 10000
                 )",
            )
            .execute(&shared.state.db.pool)
            .await
            {
                tracing::warn!(%error, "failed to prune authentication audit events");
            }
        }
        Err(error) => {
            tracing::warn!(%error, %trace_id, "failed to persist authentication audit event");
        }
    }
}

pub async fn audit_authorization_denied(
    shared: &SharedState,
    principal: &AuthenticatedPrincipal,
    method: &str,
    path: &str,
    origin: Option<String>,
    error_code: &str,
) -> String {
    let trace_id = uuid::Uuid::new_v4().to_string();
    let context = AuditContext {
        method: method.to_string(),
        path: path.to_string(),
        origin,
    };
    audit_denied(
        shared,
        &context,
        Some(principal),
        "denied",
        error_code,
        &trace_id,
    )
    .await;
    trace_id
}

async fn denied_response(
    shared: &SharedState,
    context: &AuditContext,
    principal: Option<&AuthenticatedPrincipal>,
    status: StatusCode,
    code: &'static str,
    message: &'static str,
) -> Response {
    let trace_id = uuid::Uuid::new_v4().to_string();
    audit_denied(shared, context, principal, "denied", code, &trace_id).await;
    tracing::warn!(
        %trace_id,
        method = %context.method,
        path = %context.path,
        error_code = code,
        "request access denied"
    );
    let mut response = HttpError::new(
        status,
        code,
        message,
        status == StatusCode::TOO_MANY_REQUESTS,
    )
        .with_trace_id(trace_id)
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"CrowAPI\""),
        );
    }
    response
}

pub async fn enforce_access_policy(
    State(auth): State<AuthLayerState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    let Some(policy) = access_policy(request.uri().path()) else {
        return next.run(request).await;
    };
    let audit = AuditContext::from_request(&request);
    if auth.remote_access_enabled
        && !auth
            .source_limiter
            .check(resolve_source_ip(&request, &auth.trusted_proxy_cidrs), policy, Instant::now())
    {
        let mut response = denied_response(
            &auth.shared,
            &audit,
            None,
            StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMIT_EXCEEDED",
            "来源请求频率超过限制",
        )
        .await;
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        return response;
    }
    let principal = match authenticate_api_key(&auth.shared.state.db.pool, request.headers()).await {
        Ok(principal) => principal,
        Err(AuthenticationFailure::Missing) => {
            return denied_response(
                &auth.shared,
                &audit,
                None,
                StatusCode::UNAUTHORIZED,
                "MISSING_API_KEY",
                "缺少 API Key",
            )
            .await;
        }
        Err(AuthenticationFailure::Invalid) => {
            return denied_response(
                &auth.shared,
                &audit,
                None,
                StatusCode::UNAUTHORIZED,
                "INVALID_API_KEY",
                "API Key 无效",
            )
            .await;
        }
        Err(AuthenticationFailure::Expired(principal)) => {
            return denied_response(
                &auth.shared,
                &audit,
                Some(&principal),
                StatusCode::UNAUTHORIZED,
                "API_KEY_EXPIRED",
                "API Key 已过期",
            )
            .await;
        }
        Err(AuthenticationFailure::Configuration(error)) => {
            let trace_id = uuid::Uuid::new_v4().to_string();
            audit_denied(
                &auth.shared,
                &audit,
                None,
                "error",
                "API_KEY_CONFIGURATION_INVALID",
                &trace_id,
            )
            .await;
            tracing::error!(%trace_id, %error, "invalid API key configuration");
            return HttpError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "API_KEY_CONFIGURATION_INVALID",
                "API Key 权限配置无效",
                true,
            )
            .with_trace_id(trace_id)
            .into_response();
        }
        Err(AuthenticationFailure::Repository(error)) => {
            let trace_id = uuid::Uuid::new_v4().to_string();
            audit_denied(
                &auth.shared,
                &audit,
                None,
                "error",
                "API_KEY_LOOKUP_FAILED",
                &trace_id,
            )
            .await;
            tracing::error!(%trace_id, %error, "API key lookup failed");
            return HttpError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "API_KEY_LOOKUP_FAILED",
                "读取 API Key 失败",
                true,
            )
            .with_trace_id(trace_id)
            .into_response();
        }
    };
    if !principal.allows(policy.required_scope()) {
        return denied_response(
            &auth.shared,
            &audit,
            Some(&principal),
            StatusCode::FORBIDDEN,
            "ACCESS_SCOPE_REQUIRED",
            "API Key 没有访问此端点所需的权限",
        )
        .await;
    }
    if !auth
        .limiter
        .check(&principal.api_key.id, policy, Instant::now())
    {
        let mut response = denied_response(
            &auth.shared,
            &audit,
            Some(&principal),
            StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMIT_EXCEEDED",
            "请求频率超过限制",
        )
        .await;
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        return response;
    }

    tracing::debug!(
        principal_id = %principal.api_key.id,
        access_group = policy.label(),
        method = %audit.method,
        path = %audit.path,
        "request authenticated"
    );
    request.extensions_mut().insert(principal);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::{
        access_policy, authenticate_api_key, AccessPolicy, AuthenticationFailure,
        resolve_source_ip, PrincipalRateLimiter, SourceRateLimiter, RATE_WINDOW,
    };
    use crate::config::IpCidr;
    use crate::db::models::CreateApiKeyInput;
    use crate::db::repository::Repository;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{header, HeaderMap, HeaderValue, Request};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    #[test]
    fn route_policy_is_public_only_for_health() {
        assert_eq!(access_policy("/health"), None);
        assert_eq!(access_policy("/health/live"), None);
        assert_eq!(access_policy("/health/ready"), None);
        assert_eq!(access_policy("/v1/models"), Some(AccessPolicy::Gateway));
        assert_eq!(access_policy("/api/kb"), Some(AccessPolicy::Admin));
        assert_eq!(access_policy("/mcp/sse"), Some(AccessPolicy::Mcp));
        assert_eq!(access_policy("/future-route"), Some(AccessPolicy::Admin));
    }

    #[test]
    fn rate_limiter_is_scoped_by_principal_and_policy() {
        let limiter = PrincipalRateLimiter::default();
        let started = Instant::now();
        for _ in 0..AccessPolicy::Admin.rate_limit() {
            assert!(limiter.check("key-1", AccessPolicy::Admin, started));
        }
        assert!(!limiter.check("key-1", AccessPolicy::Admin, started));
        assert!(limiter.check("key-2", AccessPolicy::Admin, started));
        assert!(limiter.check("key-1", AccessPolicy::Mcp, started));
        assert!(limiter.check(
            "key-1",
            AccessPolicy::Admin,
            started + RATE_WINDOW + Duration::from_millis(1),
        ));
    }

    #[test]
    fn source_rate_limiter_is_scoped_by_ip_and_policy() {
        let limiter = SourceRateLimiter::default();
        let started = Instant::now();
        let source = "192.0.2.10".parse().unwrap();
        for _ in 0..super::REMOTE_AUTH_REQUESTS_PER_WINDOW {
            assert!(limiter.check(source, AccessPolicy::Gateway, started));
        }
        assert!(!limiter.check(source, AccessPolicy::Gateway, started));
        assert!(limiter.check("192.0.2.11".parse().unwrap(), AccessPolicy::Gateway, started));
        assert!(limiter.check(source, AccessPolicy::Mcp, started));
        assert!(limiter.check(source, AccessPolicy::Gateway, started + RATE_WINDOW));
    }

    #[test]
    fn forwarded_headers_are_used_only_from_a_trusted_peer() {
        let trusted = vec![IpCidr::parse("10.0.0.0/8").unwrap()];
        let mut request = Request::new(Body::empty());
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from((
            [192, 0, 2, 10], 443,
        ))));
        request.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.8"),
        );
        assert_eq!(
            resolve_source_ip(&request, &trusted),
            "192.0.2.10".parse::<std::net::IpAddr>().unwrap()
        );

        request.extensions_mut().insert(ConnectInfo(SocketAddr::from((
            [10, 1, 2, 3], 443,
        ))));
        assert_eq!(
            resolve_source_ip(&request, &trusted),
            "198.51.100.8".parse::<std::net::IpAddr>().unwrap()
        );
    }

    #[test]
    fn missing_connect_info_falls_back_to_an_unspecified_source() {
        let request = Request::new(Body::empty());
        assert_eq!(
            resolve_source_ip(&request, &[]),
            "0.0.0.0".parse::<std::net::IpAddr>().unwrap()
        );
    }

    #[tokio::test]
    async fn authentication_uses_hashed_keys_and_persisted_access_scopes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create auth test database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply auth migrations");
        let created = Repository::new(pool.clone())
            .create_api_key(&CreateApiKeyInput {
                name: "admin test".to_string(),
                allowed_models: None,
                allowed_channels: None,
                access_scopes: Some(vec!["admin".to_string(), "mcp:write".to_string()]),
                quota_limit: Some(0),
                expires_at: None,
            })
            .await
            .expect("create scoped key");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", created.key)).unwrap(),
        );

        let principal = authenticate_api_key(&pool, &headers)
            .await
            .expect("authenticate scoped key");
        assert_eq!(principal.api_key.id, created.id);
        assert!(principal.allows("admin"));
        assert!(principal.allows("mcp:read"));
        assert!(principal.allows("mcp:write"));
        assert!(!principal.allows("gateway"));

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer sk-crowapi-invalid"),
        );
        assert!(matches!(
            authenticate_api_key(&pool, &headers).await,
            Err(AuthenticationFailure::Invalid)
        ));
    }
}
