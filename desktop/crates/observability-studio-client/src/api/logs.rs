//! Logs view types + typed `OrgClient` methods.
//!
//! Mirrors `apps/web/components/services/observability-service.ts` and the
//! Hono routes at `packages/backend/src/routes/observability/logs-query.ts`
//! 1:1 so updates to the canonical browser dashboard apply here with minimal
//! translation cost.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{ApiError, OrgClient};

impl<'a> OrgClient<'a> {
    /// `POST /logs/query` — paginated full-text + facet search across
    /// CloudWatch logs for the active org.
    pub async fn query_logs(&self, params: &LogQuery) -> Result<LogQueryResult, ApiError> {
        self.post("logs/query", params).await
    }

    /// `GET /logs/facets` — values for the level / log-group / function-name
    /// dropdowns. Cheap to fetch; consumers can refresh on org switch.
    pub async fn log_facets(&self) -> Result<LogFacets, ApiError> {
        self.get::<LogFacets, ()>("logs/facets", None).await
    }

    /// `GET /logs/stats?start=…&end=…` — the KPI tile aggregates (totals,
    /// per-level counts, error-rate time series, p50/p95/p99 latency).
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
        self.get("logs/stats", Some(&Q { start: &start_s, end: &end_s }))
            .await
    }
}

// ----- Request types ---------------------------------------------------------

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

// ----- Response types --------------------------------------------------------

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LogEntry {
    pub timestamp: String,
    pub organization_id: String,
    #[serde(default)]
    pub aws_account_id: Option<String>,
    #[serde(default)]
    pub log_group: Option<String>,
    #[serde(default)]
    pub log_stream: Option<String>,
    pub message: String,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub function_name: Option<String>,
    #[serde(default)]
    pub http_method: Option<String>,
    #[serde(default)]
    pub http_path: Option<String>,
    #[serde(default)]
    pub http_status: Option<i64>,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub is_json: Option<i32>,
    #[serde(default)]
    pub parsed_fields: Option<HashMap<String, String>>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub ingested_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogQueryResult {
    pub data: Vec<LogEntry>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LogFacets {
    #[serde(default)]
    pub levels: Vec<String>,
    #[serde(default)]
    pub log_groups: Vec<String>,
    #[serde(default)]
    pub function_names: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogStats {
    pub total_logs: u64,
    #[serde(default)]
    pub logs_by_level: Vec<LevelCount>,
    #[serde(default)]
    pub error_rate_time_series: Vec<RatePoint>,
    #[serde(default)]
    pub top_log_groups: Vec<GroupCount>,
    #[serde(default)]
    pub top_errors: Vec<ErrorCount>,
    pub duration_percentiles: Percentiles,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LevelCount {
    pub level: String,
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RatePoint {
    pub bucket: String,
    pub total: u64,
    pub errors: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupCount {
    pub log_group: String,
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorCount {
    pub error: String,
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

// ----- Helpers --------------------------------------------------------------

impl LogStats {
    /// Sum the per-level counts that count as "errors" (ERROR + FATAL).
    pub fn errors_total(&self) -> u64 {
        self.logs_by_level
            .iter()
            .filter(|l| matches!(l.level.to_ascii_uppercase().as_str(), "ERROR" | "FATAL"))
            .map(|l| l.count)
            .sum()
    }

    /// Error rate as a 0–100 percentage. Returns 0.0 when `total_logs == 0`.
    pub fn error_rate_pct(&self) -> f64 {
        if self.total_logs == 0 {
            0.0
        } else {
            self.errors_total() as f64 * 100.0 / self.total_logs as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn log_entry_decodes_minimal_payload() {
        // Server returns sparse rows when CloudWatch isn't structured JSON.
        // Verify our serde defaults absorb the missing fields without panicking.
        let row: LogEntry = serde_json::from_value(json!({
            "timestamp": "2026-05-23T19:00:00Z",
            "organization_id": "8be5f5fd-cf71-43ba-9df9-01e15acdaf8e",
            "message": "boot complete",
        }))
        .expect("minimal LogEntry decodes");
        assert_eq!(row.message, "boot complete");
        assert!(row.level.is_none());
        assert!(row.parsed_fields.is_none());
    }

    #[test]
    fn log_stats_error_rate_computation() {
        let stats = LogStats {
            total_logs: 100,
            logs_by_level: vec![
                LevelCount { level: "INFO".into(), count: 90 },
                LevelCount { level: "ERROR".into(), count: 8 },
                LevelCount { level: "FATAL".into(), count: 2 },
            ],
            error_rate_time_series: vec![],
            top_log_groups: vec![],
            top_errors: vec![],
            duration_percentiles: Percentiles { p50: 12.0, p95: 60.0, p99: 110.0 },
        };
        assert_eq!(stats.errors_total(), 10);
        assert!((stats.error_rate_pct() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn log_stats_zero_division_is_safe() {
        let stats = LogStats {
            total_logs: 0,
            logs_by_level: vec![],
            error_rate_time_series: vec![],
            top_log_groups: vec![],
            top_errors: vec![],
            duration_percentiles: Percentiles::default(),
        };
        assert_eq!(stats.error_rate_pct(), 0.0);
    }

    #[test]
    fn log_query_skips_none_fields() {
        // The backend rejects payloads with explicit `null` for optional
        // filters; verify serde drops them entirely.
        let q = LogQuery {
            search: Some("timeout".into()),
            level: None,
            log_group: None,
            function_name: None,
            http_path: None,
            http_status: None,
            trace_id: None,
            time_range: TimeRange {
                start: "2026-05-23T18:00:00Z".parse().unwrap(),
                end: "2026-05-23T19:00:00Z".parse().unwrap(),
            },
            limit: Some(100),
            offset: Some(0),
            order_by: Some("desc".into()),
        };
        let s = serde_json::to_string(&q).unwrap();
        assert!(s.contains("\"search\":\"timeout\""));
        assert!(!s.contains("\"level\""));
        assert!(!s.contains("\"log_group\""));
        assert!(s.contains("\"limit\":100"));
    }
}
