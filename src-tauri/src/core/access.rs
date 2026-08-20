pub fn parse_scope(value: &str) -> Result<Vec<String>, serde_json::Error> {
    serde_json::from_str(value)
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
}
