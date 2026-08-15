pub fn parse_scope(value: &str) -> Result<Vec<String>, serde_json::Error> {
    serde_json::from_str(value)
}

pub fn scope_allows(scope: &[String], value: &str, all_label: &str) -> bool {
    scope.is_empty() || scope.iter().any(|item| item == "*" || item == all_label || item == value)
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
}
