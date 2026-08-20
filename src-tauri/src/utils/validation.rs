pub fn normalize_http_base_url(value: &str) -> Result<String, &'static str> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("API 地址不能为空");
    }

    let parsed = reqwest::Url::parse(trimmed).map_err(|_| "API 地址格式无效")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("API 地址仅支持 HTTP 或 HTTPS");
    }
    if parsed.host_str().is_none() {
        return Err("API 地址缺少主机名");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("API 地址不能包含用户名或密码");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("API 地址不能包含查询参数或片段");
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_http_base_url;

    #[test]
    fn accepts_remote_and_local_http_endpoints() {
        assert_eq!(
            normalize_http_base_url(" https://api.example.com/v1/ ").unwrap(),
            "https://api.example.com/v1",
        );
        assert_eq!(
            normalize_http_base_url("http://127.0.0.1:11434/v1").unwrap(),
            "http://127.0.0.1:11434/v1",
        );
    }

    #[test]
    fn rejects_credentials_queries_and_unsupported_schemes() {
        assert!(normalize_http_base_url("file:///tmp/api").is_err());
        assert!(normalize_http_base_url("https://user:secret@example.com/v1").is_err());
        assert!(normalize_http_base_url("https://example.com/v1?token=secret").is_err());
    }
}
