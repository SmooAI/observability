//! `POST /organizations/{org_id}/observability/logs/query`,
//! `GET .../logs/facets`, `GET .../logs/stats`.
//!
//! Types mirror `apps/web/components/services/observability-service.ts` and
//! `packages/backend/src/routes/observability/logs-query.ts`.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{ApiError, OrgClient};

impl<'a> OrgClient<'a> {
    pub async fn query_logs(&self, params: &LogQuery) -> Result<LogQueryResult, ApiError> {
        self.post("logs/query", params).await
    }

    pub async fn log_facets(&self) -> Result<LogFacets, ApiError> {
        self.get("logs/facets", Option::<&()>::None).await
    }

    pub async fn log_stats(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<LogStats, ApiError> {
        #[derive(Serialize)]
        struct Q<'a> {
            start: &'a str,
            end: &'a str,
        }
        let start_s = start.to_rfc3339();
        let end_s = end.to_rfc3339();
        self.get("logs/stats", Some(&Q { start: &start_s, end: &end_s })).await
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub time_range: TimeRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub organization_id: String,
    pub aws_account_id: Option<String>,
    pub log_group: Option<String>,
    pub log_stream: Option<String>,
    pub message: String,
    pub level: Option<String>,
    pub request_id: Option<String>,
    pub function_name: Option<String>,
    pub http_method: Option<String>,
    pub http_path: Option<String>,
    pub http_status: Option<i64>,
    pub duration_ms: Option<f64>,
    pub error: Option<String>,
    pub user_id: Option<String>,
    pub trace_id: Option<String>,
    pub is_json: Option<i32>,
    pub parsed_fields: Option<HashMap<String, String>>,
    pub raw: Option<String>,
    pub ingested_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogQueryResult {
    pub data: Vec<LogEntry>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogFacets {
    pub levels: Vec<String>,
    pub log_groups: Vec<String>,
    pub function_names: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogStats {
    pub total_logs: u64,
    pub logs_by_level: Vec<LevelCount>,
    pub error_rate_time_series: Vec<RatePoint>,
    pub top_log_groups: Vec<GroupCount>,
    pub top_errors: Vec<ErrorCount>,
    pub duration_percentiles: Percentiles,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LevelCount { pub level: String, pub count: u64 }

#[derive(Debug, Clone, Deserialize)]
pub struct RatePoint { pub bucket: String, pub total: u64, pub errors: u64 }

#[derive(Debug, Clone, Deserialize)]
pub struct GroupCount { pub log_group: String, pub count: u64 }

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorCount { pub error: String, pub count: u64 }

#[derive(Debug, Clone, Deserialize)]
pub struct Percentiles { pub p50: f64, pub p95: f64, pub p99: f64 }
