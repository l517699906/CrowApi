use crate::db::models::Channel;
use crate::db::repository::Repository;

/// Call CrowAPI's internal channel dispatch to get embeddings.
/// Reuses existing channel config (base_url, api_key, model_mapping) but
/// sends requests directly to the /embeddings endpoint instead of /chat/completions,
/// because all adaptors hard-code the chat completions URL.
pub async fn embed(
    texts: &[String],
    model: &str,
    repo: &Repository,
) -> Result<Vec<Vec<f32>>, String> {
    embed_with_channel(texts, model, repo, None).await
}

pub async fn embed_with_channel(
    texts: &[String],
    model: &str,
    repo: &Repository,
    preferred_channel_id: Option<&str>,
) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(vec![]);
    }

    // Get enabled channels
    let channels = repo
        .get_enabled_channels()
        .await
        .map_err(|e| format!("Failed to get channels: {}", e))?;

    let candidates = if let Some(channel_id) = preferred_channel_id.filter(|id| !id.is_empty()) {
        let channel = channels
            .iter()
            .find(|channel| channel.id == channel_id)
            .ok_or_else(|| format!("Configured embedding channel is unavailable: {}", channel_id))?;
        if !supports_embeddings(channel) {
            return Err(format!(
                "Configured channel {} is not OpenAI-compatible and cannot provide embeddings",
                channel.name,
            ));
        }
        vec![channel.clone()]
    } else {
        Dispatcher::select_channels(&channels, model)
            .into_iter()
            .filter(|channel| supports_embeddings(channel))
            .collect()
    };

    if candidates.is_empty() {
        return Err(format!(
            "No OpenAI-compatible channel declares embedding model: {}",
            model,
        ));
    }

    for channel in &candidates {
        match try_embed_with_channel(texts, model, channel).await {
            Ok(embeddings) => {
                // Log success and validate dimensions
                if !embeddings.is_empty() {
                    tracing::info!(
                        "Embedding success: channel={}, model={}, texts={}, dim={}",
                        channel.name, model, texts.len(), embeddings[0].len()
                    );
                }
                return Ok(embeddings);
            }
            Err(e) => {
                tracing::warn!(
                    "Embedding failed on channel {} (model={}): {} — trying next channel",
                    channel.name, model, e
                );
                continue;
            }
        }
    }

    Err(format!(
        "All channels failed for embedding model: {}. Make sure at least one channel supports embeddings.",
        model
    ))
}

async fn try_embed_with_channel(
    texts: &[String],
    model: &str,
    channel: &Channel,
) -> Result<Vec<Vec<f32>>, String> {
    let base_url = channel.base_url.trim_end_matches('/');

    // Apply model mapping if configured
    let mapping = serde_json::from_str(&channel.model_mapping).unwrap_or_default();
    let actual_model = crate::adaptor::resolve_model_mapping(model, &mapping);

    let url = format!("{}/embeddings", base_url);
    let body = serde_json::json!({
        "model": actual_model,
        "input": texts,
        "encoding_format": "float"
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", channel.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(channel.timeout_secs.max(1) as u64))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status, text.chars().take(300).collect::<String>()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("Parse response failed: {}", e))?;

    let data = json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or("Invalid embedding response: missing data array")?;

    let embeddings: Vec<Vec<f32>> = data
        .iter()
        .filter_map(|item| {
            item.get("embedding")
                .and_then(|e| e.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect()
                })
        })
        .collect();

    if embeddings.len() != texts.len() {
        return Err(format!(
            "Embedding count mismatch: expected {}, got {}",
            texts.len(),
            embeddings.len()
        ));
    }

    // Validate all embeddings have same dimension
    if !embeddings.is_empty() {
        let dim = embeddings[0].len();
        for (i, emb) in embeddings.iter().enumerate().skip(1) {
            if emb.len() != dim {
                return Err(format!(
                    "Inconsistent embedding dimensions: item 0 has dim {}, item {} has dim {}",
                    dim, i, emb.len()
                ));
            }
        }
        tracing::debug!(
            "Embeddings validated: {} items, dim {}",
            embeddings.len(), dim
        );
    }

    Ok(embeddings)
}

fn supports_embeddings(channel: &Channel) -> bool {
    matches!(channel.channel_type.as_str(), "openai" | "custom")
}

// Re-export Dispatcher for select_channels
use crate::core::dispatcher::Dispatcher;

#[cfg(test)]
mod tests {
    use super::supports_embeddings;
    use crate::db::models::Channel;

    fn channel(channel_type: &str) -> Channel {
        Channel {
            id: "channel-1".to_string(),
            name: "test".to_string(),
            channel_type: channel_type.to_string(),
            base_url: "https://example.test/v1".to_string(),
            api_key: "secret".to_string(),
            secret_ref: None,
            api_key_last4: "cret".to_string(),
            models: "[]".to_string(),
            status: 1,
            priority: 0,
            weight: 1,
            config: "{}".to_string(),
            model_mapping: "{}".to_string(),
            timeout_secs: 60,
            created_at: String::new(),
            updated_at: String::new(),
            last_test_at: None,
            last_test_ok: None,
        }
    }

    #[test]
    fn only_openai_compatible_channels_are_embedding_candidates() {
        assert!(supports_embeddings(&channel("openai")));
        assert!(supports_embeddings(&channel("custom")));
        assert!(!supports_embeddings(&channel("claude")));
        assert!(!supports_embeddings(&channel("gemini")));
        assert!(!supports_embeddings(&channel("deepseek")));
    }
}
