use async_trait::async_trait;
use super::*;

pub struct GeminiAdaptor;

#[async_trait]
impl Adaptor for GeminiAdaptor {
    fn channel_type(&self) -> &'static str { "gemini" }
    fn default_models(&self) -> Vec<&'static str> { vec!["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"] }
    fn default_base_url(&self) -> &str { "https://generativelanguage.googleapis.com" }

    async fn test(&self, config: &ChannelConfig) -> Result<TestResult, anyhow::Error> {
        let start = std::time::Instant::now();
        let model = config.models.first().map(|s| s.as_str()).unwrap_or("gemini-2.0-flash");
        let url = format!("{}/v1beta/models/{}", config.base_url.trim_end_matches('/'), model);
        let client = reqwest::Client::new();
        match client
            .get(&url)
            .header("x-goog-api-key", &config.api_key)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(r) => {
                let latency = start.elapsed().as_millis() as u64;
                if r.status().is_success() {
                    Ok(TestResult { success: true, message: "连接成功".to_string(), latency_ms: latency })
                } else {
                    Ok(TestResult { success: false, message: format!("HTTP {}", r.status()), latency_ms: latency })
                }
            }
            Err(e) => Ok(TestResult { success: false, message: format!("连接失败: {}", e), latency_ms: start.elapsed().as_millis() as u64 }),
        }
    }

    async fn forward(&self, request: &ProxyRequest, config: &ChannelConfig) -> Result<(u16, serde_json::Value, Option<TokenUsage>), anyhow::Error> {
        let model = request.body.get("model").and_then(|m| m.as_str()).unwrap_or("gemini-2.0-flash");
        // 非流式：:generateContent
        let url = format!("{}/v1beta/models/{}:generateContent", config.base_url.trim_end_matches('/'), model);

        let openai_body = &request.body;
        let gemini_body = convert_openai_to_gemini(openai_body);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &config.api_key)
            .json(&gemini_body)
            .send()
            .await?;
        let status = resp.status().as_u16();
        let gemini_json: serde_json::Value = resp.json().await?;

        let openai_response = convert_gemini_to_openai(&gemini_json, model);
        let usage = openai_response.get("usage").and_then(|u| Some(TokenUsage {
            prompt_tokens: u.get("prompt_tokens")?.as_u64()?,
            completion_tokens: u.get("completion_tokens")?.as_u64()?,
            total_tokens: u.get("total_tokens")?.as_u64()?,
        }));

        Ok((status, openai_response, usage))
    }

    async fn forward_stream(&self, request: &ProxyRequest, config: &ChannelConfig) -> Result<reqwest::Response, anyhow::Error> {
        let model = request.body.get("model").and_then(|m| m.as_str()).unwrap_or("gemini-2.0-flash");
        // 流式：:streamGenerateContent + alt=sse
        let url = format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", config.base_url.trim_end_matches('/'), model);

        let openai_body = &request.body;
        let gemini_body = convert_openai_to_gemini(openai_body);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &config.api_key)
            .json(&gemini_body)
            .send()
            .await?;
        Ok(resp)
    }
}

fn convert_openai_to_gemini(body: &serde_json::Value) -> serde_json::Value {
    let messages = body.get("messages").and_then(|m| m.as_array()).cloned().unwrap_or_default();

    let mut system_instruction = None;
    let mut contents = Vec::new();

    for msg in &messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

        if role == "system" {
            system_instruction = Some(serde_json::json!({
                "parts": [{"text": content}]
            }));
        } else {
            // Skip empty assistant messages without tool_calls
            if role == "assistant" && content.is_empty() {
                let has_tool_calls = msg
                    .get("tool_calls")
                    .and_then(|t| t.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                if !has_tool_calls {
                    continue;
                }
            }

            contents.push(serde_json::json!({
                // assistant -> model
                "role": if role == "assistant" { "model" } else { "user" },
                "parts": [{"text": content}]
            }));
        }
    }

    let mut gemini_body = serde_json::json!({
        "contents": contents,
    });

    if let Some(si) = system_instruction {
        gemini_body["systemInstruction"] = si;
    }

    // 生成参数收口到 generationConfig
    if let Some(temp) = body.get("temperature") {
        gemini_body["generationConfig"]["temperature"] = temp.clone();
    }
    if let Some(max_tokens) = body.get("max_tokens") {
        gemini_body["generationConfig"]["maxOutputTokens"] = max_tokens.clone();
    }

    gemini_body
}

fn convert_gemini_to_openai(gemini_json: &serde_json::Value, model: &str) -> serde_json::Value {
    // candidates[0].content.parts[].text 拼接
    let content = gemini_json.get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|cand| cand.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts.iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    // usageMetadata.promptTokenCount / candidatesTokenCount
    let prompt_tokens = gemini_json.get("usageMetadata").and_then(|u| u.get("promptTokenCount")).and_then(|t| t.as_u64()).unwrap_or(0);
    let completion_tokens = gemini_json.get("usageMetadata").and_then(|u| u.get("candidatesTokenCount")).and_then(|t| t.as_u64()).unwrap_or(0);

    serde_json::json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
            },
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::convert_openai_to_gemini;
    use serde_json::json;

    #[test]
    fn converts_messages_and_skips_empty_assistant_message() {
        let converted = convert_openai_to_gemini(&json!({
            "messages": [
                {"role": "system", "content": "Follow the instructions."},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": ""},
                {"role": "assistant", "content": "Hi"}
            ]
        }));

        assert_eq!(
            converted["systemInstruction"],
            json!({"parts": [{"text": "Follow the instructions."}]})
        );
        assert_eq!(
            converted["contents"],
            json!([
                {"role": "user", "parts": [{"text": "Hello"}]},
                {"role": "model", "parts": [{"text": "Hi"}]}
            ])
        );
    }
}
