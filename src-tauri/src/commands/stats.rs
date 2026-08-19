use crate::db::repository::Repository;
use crate::AppState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardStatsDto {
    pub today_requests: i64,
    pub today_total_tokens: i64,
    pub active_channels: i64,
    pub avg_latency_ms: f64,
    pub total_channels: i64,
    pub total_api_keys: i64,
    pub total_requests: i64,
    pub total_tokens: i64,
    pub total_knowledge_bases: i64,
    pub total_kb_documents: i64,
    pub total_kb_chunks: i64,
    pub protocols: Vec<ProtocolUsageStatDto>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DashboardStatsInput {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProtocolUsageStatDto {
    pub mode: String,
    pub request_count: i64,
    pub total_tokens: i64,
}

impl From<crate::db::models::ProtocolUsageStat> for ProtocolUsageStatDto {
    fn from(value: crate::db::models::ProtocolUsageStat) -> Self {
        Self {
            mode: value.mode,
            request_count: value.request_count,
            total_tokens: value.total_tokens,
        }
    }
}

#[tauri::command]
pub async fn get_dashboard_stats(
    input: DashboardStatsInput,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<DashboardStatsDto, String> {
    let repo = Repository::new(state.db.pool.clone());
    let s = repo
        .get_dashboard_stats(input.date_from.as_deref(), input.date_to.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(DashboardStatsDto {
        today_requests: s.today_requests,
        today_total_tokens: s.today_total_tokens,
        active_channels: s.active_channels,
        avg_latency_ms: s.avg_latency_ms,
        total_channels: s.total_channels,
        total_api_keys: s.total_api_keys,
        total_requests: s.total_requests,
        total_tokens: s.total_tokens,
        total_knowledge_bases: s.total_knowledge_bases,
        total_kb_documents: s.total_kb_documents,
        total_kb_chunks: s.total_kb_chunks,
        protocols: s.protocols.into_iter().map(Into::into).collect(),
    })
}

#[derive(Debug, Default, Deserialize)]
pub struct UsageStatsInput {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub bucket_seconds: Option<i64>,
    pub bucket_count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct UsageBucketStatDto {
    pub bucket_index: i64,
    pub request_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelUsageStatDto {
    pub name: String,
    pub request_count: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct ChannelUsageStatDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub request_count: i64,
}

#[derive(Debug, Serialize)]
pub struct UsageStatsDto {
    pub total_requests: i64,
    pub total_tokens: i64,
    pub failed_requests: i64,
    pub protocols: Vec<ProtocolUsageStatDto>,
    pub series: Vec<UsageBucketStatDto>,
    pub models: Vec<ModelUsageStatDto>,
    pub channels: Vec<ChannelUsageStatDto>,
}

#[tauri::command]
pub async fn get_usage_stats(
    input: UsageStatsInput,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<UsageStatsDto, String> {
    let repo = Repository::new(state.db.pool.clone());
    let bucket_seconds = input
        .bucket_seconds
        .unwrap_or(3_600)
        .clamp(60, 2_678_400);
    let bucket_count = input.bucket_count.unwrap_or(24).clamp(1, 366);
    let stats = repo
        .get_usage_stats(
            input.date_from.as_deref(),
            input.date_to.as_deref(),
            bucket_seconds,
            bucket_count,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(UsageStatsDto {
        total_requests: stats.total_requests,
        total_tokens: stats.total_tokens,
        failed_requests: stats.failed_requests,
        protocols: stats.protocols.into_iter().map(Into::into).collect(),
        series: stats
            .series
            .into_iter()
            .map(|item| UsageBucketStatDto {
                bucket_index: item.bucket_index,
                request_count: item.request_count,
            })
            .collect(),
        models: stats
            .models
            .into_iter()
            .map(|item| ModelUsageStatDto {
                name: item.name,
                request_count: item.request_count,
                total_tokens: item.total_tokens,
            })
            .collect(),
        channels: stats
            .channels
            .into_iter()
            .map(|item| ChannelUsageStatDto {
                id: item.id,
                name: item.name,
                channel_type: item.channel_type,
                request_count: item.request_count,
            })
            .collect(),
    })
}
