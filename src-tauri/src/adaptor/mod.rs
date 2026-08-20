pub mod openai;
pub mod claude;
pub mod gemini;
pub mod deepseek;
pub mod custom;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// 渠道配置——从数据库 Channel 转换而来
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub model_mapping: serde_json::Value,
    pub extra: serde_json::Value,
    pub timeout_secs: u64,
}

// 代理请求——统一的上游请求抽象
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRequest {
    pub model: String,
    pub body: serde_json::Value,
    pub stream: bool,
}

// 渠道连通性测试结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
}

// Token 用量——统一各家的计费格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[async_trait]
pub trait Adaptor: Send + Sync {
    // 渠道类型标识
    #[allow(dead_code)]
    fn channel_type(&self) -> &'static str;
    // 默认支持的模型列表
    #[allow(dead_code)]
    fn default_models(&self) -> Vec<&'static str>;
    // 默认 API 地址
    #[allow(dead_code)]
    fn default_base_url(&self) -> &str;

    // 测试渠道连通性
    async fn test(&self, config: &ChannelConfig) -> Result<TestResult, anyhow::Error>;

    // 非流式转发：返回 (状态码, 响应体, Token用量)
    async fn forward(
        &self,
        request: &ProxyRequest,
        config: &ChannelConfig,
    ) -> Result<(u16, serde_json::Value, Option<TokenUsage>), anyhow::Error>;

    // 流式转发：直接返回 reqwest::Response，由调用方逐字节转发 SSE
    async fn forward_stream(
        &self,
        request: &ProxyRequest,
        config: &ChannelConfig,
    ) -> Result<reqwest::Response, anyhow::Error>;
}

pub fn get_adaptor(channel_type: &str) -> Box<dyn Adaptor> {
    match channel_type.to_ascii_lowercase().as_str() {
        "openai" => Box::new(openai::OpenAIAdaptor),
        "deepseek" => Box::new(deepseek::DeepSeekAdaptor),
        "claude" => Box::new(claude::ClaudeAdaptor),
        "gemini" => Box::new(gemini::GeminiAdaptor),
        "custom" => Box::new(custom::CustomAdaptor),
        _ => Box::new(custom::CustomAdaptor),      // 未知类型兜底走自定义
    }
}

pub fn resolve_model_mapping(requested_model: &str, mapping: &serde_json::Value) -> String {
    let Some(mapped) = mapping.get(requested_model) else {
        return requested_model.to_string();
    };
    if let Some(model) = mapped.as_str() {
        return model.to_string();
    }
    let Some(models) = mapped.as_array() else {
        return requested_model.to_string();
    };
    let candidates = models.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>();
    if candidates.is_empty() {
        return requested_model.to_string();
    }
    let index = rand::Rng::random_range(&mut rand::rng(), 0..candidates.len());
    candidates[index].to_string()
}

/// Resolve a channel's model mapping exactly once and disable re-mapping inside
/// protocol adaptors, keeping the forwarded model and request log consistent.
pub fn prepare_channel_request(
    request: &ProxyRequest,
    config: &ChannelConfig,
) -> (ProxyRequest, ChannelConfig, String) {
    let requested_model = request
        .body
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&request.model);
    let upstream_model = resolve_model_mapping(requested_model, &config.model_mapping);
    let mut channel_request = request.clone();
    channel_request.model = upstream_model.clone();
    if channel_request.body.is_object() {
        channel_request.body["model"] = serde_json::Value::String(upstream_model.clone());
    }
    let mut channel_config = config.clone();
    channel_config.model_mapping = serde_json::Value::Object(Default::default());
    (channel_request, channel_config, upstream_model)
}

#[cfg(test)]
mod model_mapping_tests {
    use super::{prepare_channel_request, ChannelConfig, ProxyRequest};

    #[test]
    fn array_mapping_updates_body_and_log_model_together() {
        let request = ProxyRequest {
            model: "public-model".to_string(),
            body: serde_json::json!({"model": "public-model", "messages": []}),
            stream: true,
        };
        let config = ChannelConfig {
            base_url: "http://localhost".to_string(),
            api_key: "test".to_string(),
            models: vec![],
            model_mapping: serde_json::json!({"public-model": ["upstream-model"]}),
            extra: serde_json::json!({}),
            timeout_secs: 1,
        };

        let (mapped, mapped_config, upstream_model) = prepare_channel_request(&request, &config);
        assert_eq!(upstream_model, "upstream-model");
        assert_eq!(mapped.model, "upstream-model");
        assert_eq!(mapped.body["model"], "upstream-model");
        assert_eq!(mapped_config.model_mapping, serde_json::json!({}));
    }
}

#[allow(dead_code)]
pub fn channel_types() -> Vec<ChannelTypeInfo> {
    vec![
        ChannelTypeInfo { value: "openai", label: "OpenAI", category: "international", default_base_url: "https://api.openai.com/v1", models: vec!["gpt-5.4", "gpt-5.5", "gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-3.5-turbo"] },
        ChannelTypeInfo { value: "deepseek", label: "DeepSeek", category: "international", default_base_url: "https://api.deepseek.com/v1", models: vec!["deepseek-chat", "deepseek-coder", "deepseek-reasoner"] },
        ChannelTypeInfo { value: "claude", label: "Anthropic Claude", category: "international", default_base_url: "https://api.anthropic.com/v1", models: vec!["claude-sonnet-4-20250514", "claude-3-7-sonnet-20250219", "claude-3-5-haiku-20241022"] },
        ChannelTypeInfo { value: "gemini", label: "Google Gemini", category: "international", default_base_url: "https://generativelanguage.googleapis.com", models: vec!["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"] },
        ChannelTypeInfo { value: "qwen", label: "通义千问", category: "domestic", default_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", models: vec!["qwen-max", "qwen-plus", "qwen-turbo"] },
        ChannelTypeInfo { value: "zhipu", label: "智谱 GLM", category: "domestic", default_base_url: "https://open.bigmodel.cn/api/paas/v4", models: vec!["glm-4-plus", "glm-4-flash", "glm-4-air"] },
        ChannelTypeInfo { value: "moonshot", label: "Moonshot AI", category: "domestic", default_base_url: "https://api.moonshot.cn/v1", models: vec!["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"] },
        ChannelTypeInfo { value: "doubao", label: "字节豆包", category: "domestic", default_base_url: "https://ark.cn-beijing.volces.com/api/v3", models: vec!["doubao-pro-32k", "doubao-pro-128k", "doubao-lite-32k"] },
        ChannelTypeInfo { value: "ollama", label: "Ollama (本地)", category: "local", default_base_url: "http://localhost:11434/v1", models: vec!["llama3.1", "qwen2.5", "mistral"] },
        ChannelTypeInfo { value: "custom", label: "自定义", category: "custom", default_base_url: "", models: vec![] },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ChannelTypeInfo {
    pub value: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub default_base_url: &'static str,
    pub models: Vec<&'static str>,
}
