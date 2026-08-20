use serde::Serialize;
use std::fmt::{Display, Formatter};

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            trace_id: None,
            retryable,
            details: None,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("VALIDATION_ERROR", message, false)
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, message, false)
    }

    pub fn reported(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        source: impl Display,
    ) -> Self {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let code = code.into();
        let message = message.into();
        tracing::error!(trace_id = %trace_id, error_code = %code, error = %source, "{}", message);

        Self {
            code,
            message,
            trace_id: Some(trace_id),
            retryable,
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl Display for CommandError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::new("OPERATION_FAILED", message, false)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

pub trait CommandResultExt<T> {
    fn command_error(
        self,
        code: &'static str,
        message: &'static str,
        retryable: bool,
    ) -> CommandResult<T>;
}

impl<T, E> CommandResultExt<T> for Result<T, E>
where
    E: Display,
{
    fn command_error(
        self,
        code: &'static str,
        message: &'static str,
        retryable: bool,
    ) -> CommandResult<T> {
        self.map_err(|source| CommandError::reported(code, message, retryable, source))
    }
}

#[cfg(test)]
mod tests {
    use super::CommandError;

    #[test]
    fn validation_error_has_stable_shape_without_trace() {
        let value = serde_json::to_value(CommandError::validation("参数错误"))
            .expect("serialize command error");

        assert_eq!(value["code"], "VALIDATION_ERROR");
        assert_eq!(value["message"], "参数错误");
        assert_eq!(value["retryable"], false);
        assert!(value.get("trace_id").is_none());
        assert!(value.get("details").is_none());
    }

    #[test]
    fn reported_error_gets_trace_id_without_serializing_source() {
        let error = CommandError::reported(
            "DATABASE_ERROR",
            "读取失败",
            true,
            "sqlite contained a sensitive path",
        );
        let value = serde_json::to_value(error).expect("serialize command error");

        assert_eq!(value["code"], "DATABASE_ERROR");
        assert_eq!(value["message"], "读取失败");
        assert_eq!(value["retryable"], true);
        assert!(value["trace_id"].as_str().is_some_and(|id| !id.is_empty()));
        assert!(!value.to_string().contains("sensitive path"));
    }
}
