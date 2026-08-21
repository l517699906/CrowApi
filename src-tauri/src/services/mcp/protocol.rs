use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct McpRequest {
    pub(crate) jsonrpc: String,
    pub(crate) id: Option<serde_json::Value>,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(Debug, Serialize)]
struct McpError {
    code: i32,
    message: String,
}

impl McpResponse {
    pub(crate) fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub(crate) fn error(id: Option<serde_json::Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(McpError { code, message }),
        }
    }

    pub(crate) fn to_json_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

pub(crate) fn validate_jsonrpc_request(req: &McpRequest) -> Result<(), McpResponse> {
    if req.jsonrpc != "2.0" || req.method.trim().is_empty() {
        return Err(McpResponse::error(
            req.id.clone(),
            -32600,
            "Invalid Request".to_string(),
        ));
    }
    Ok(())
}
