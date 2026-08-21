pub fn parse_scope(value: &str) -> Result<Vec<String>, serde_json::Error> {
    serde_json::from_str(value)
}

pub const ACCESS_SCOPE_GATEWAY: &str = "gateway";
pub const ACCESS_SCOPE_MCP_READ: &str = "mcp:read";
pub const ACCESS_SCOPE_MCP_WRITE: &str = "mcp:write";
pub const ACCESS_SCOPE_ADMIN: &str = "admin";

pub const SUPPORTED_ACCESS_SCOPES: &[&str] = &[
    ACCESS_SCOPE_GATEWAY,
    ACCESS_SCOPE_MCP_READ,
    ACCESS_SCOPE_MCP_WRITE,
    ACCESS_SCOPE_ADMIN,
];

pub fn normalize_access_scopes(scopes: Option<&[String]>) -> Result<Vec<String>, String> {
    let requested = scopes
        .map(<[String]>::to_vec)
        .unwrap_or_else(|| vec![ACCESS_SCOPE_GATEWAY.to_string()]);
    if requested.is_empty() {
        return Err("访问密钥至少需要一个权限范围".to_string());
    }

    let mut normalized = Vec::new();
    for supported in SUPPORTED_ACCESS_SCOPES {
        if requested.iter().any(|scope| scope == supported) {
            normalized.push((*supported).to_string());
        }
    }
    if normalized.len() != requested.iter().collect::<std::collections::HashSet<_>>().len() {
        return Err("访问密钥包含不支持的权限范围".to_string());
    }
    Ok(normalized)
}

pub fn parse_access_scopes(value: &str) -> Result<Vec<String>, String> {
    let scopes: Vec<String> = serde_json::from_str(value)
        .map_err(|_| "访问密钥权限配置不是有效的 JSON 数组".to_string())?;
    normalize_access_scopes(Some(&scopes))
}

pub fn access_scope_allows(scopes: &[String], required: &str) -> bool {
    scopes.iter().any(|scope| scope == required)
        || (required == ACCESS_SCOPE_MCP_READ
            && scopes.iter().any(|scope| scope == ACCESS_SCOPE_MCP_WRITE))
}

pub fn scope_allows(scope: &[String], value: &str, all_label: &str) -> bool {
    scope.is_empty() || scope.iter().any(|item| item == "*" || item == all_label || item == value)
}

pub fn api_key_is_expired(expires_at: Option<&str>) -> Result<bool, chrono::ParseError> {
    let Some(expires_at) = expires_at.filter(|value| !value.trim().is_empty()) else {
        return Ok(false);
    };
    let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at)?;
    Ok(expires_at.with_timezone(&chrono::Utc) <= chrono::Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_supports_all_and_exact_values() {
        assert!(scope_allows(&[], "deepseek-chat", "全部模型"));
        assert!(scope_allows(&["*".to_string()], "deepseek-chat", "全部模型"));
        assert!(scope_allows(&["全部模型".to_string()], "deepseek-chat", "全部模型"));
        assert!(scope_allows(&["deepseek-chat".to_string()], "deepseek-chat", "全部模型"));
        assert!(!scope_allows(&["gpt-4o".to_string()], "deepseek-chat", "全部模型"));
    }

    #[test]
    fn malformed_scope_is_rejected() {
        assert!(parse_scope("not-json").is_err());
    }

    #[test]
    fn expiration_is_enforced() {
        let expired = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        let active = (chrono::Utc::now() + chrono::Duration::minutes(1)).to_rfc3339();
        assert!(api_key_is_expired(Some(&expired)).unwrap());
        assert!(!api_key_is_expired(Some(&active)).unwrap());
        assert!(!api_key_is_expired(None).unwrap());
    }

    #[test]
    fn access_scopes_are_canonical_and_write_implies_mcp_read() {
        let scopes = normalize_access_scopes(Some(&[
            ACCESS_SCOPE_ADMIN.to_string(),
            ACCESS_SCOPE_MCP_WRITE.to_string(),
            ACCESS_SCOPE_ADMIN.to_string(),
        ]))
        .unwrap();
        assert_eq!(
            scopes,
            vec![ACCESS_SCOPE_MCP_WRITE.to_string(), ACCESS_SCOPE_ADMIN.to_string()]
        );
        assert!(access_scope_allows(&scopes, ACCESS_SCOPE_MCP_READ));
        assert!(!access_scope_allows(&scopes, ACCESS_SCOPE_GATEWAY));
    }

    #[test]
    fn access_scopes_default_to_gateway_and_reject_unknown_values() {
        assert_eq!(
            normalize_access_scopes(None).unwrap(),
            vec![ACCESS_SCOPE_GATEWAY.to_string()]
        );
        assert!(normalize_access_scopes(Some(&[])).is_err());
        assert!(normalize_access_scopes(Some(&["unknown".to_string()])).is_err());
    }
}
