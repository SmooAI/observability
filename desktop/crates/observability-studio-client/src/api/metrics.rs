//! Metrics view types + typed `OrgClient` methods.
//!
//! Mirrors `packages/backend/src/routes/observability/metrics-query.ts` —
//! list descriptors, fetch a timeseries (mean / percentiles / heatmap modes),
//! enumerate attribute keys for the group-by dropdown.

use serde::{Deserialize, Serialize};

use super::{ApiError, OrgClient};

impl<'a> OrgClient<'a> {
    /// `GET /metrics` — all metric descriptors (name, kind, unit, service).
    pub async fn list_metrics(
        &self,
        params: &MetricListParams,
    ) -> Result<MetricDescriptorList, ApiError> {
        self.get("metrics", Some(params)).await
    }

    /// `GET /metrics/timeseries?...&mode={mean|percentiles|heatmap}`. The
    /// `TimeseriesPayload` enum picks the right variant via `untagged` matching
    /// on the inner field shape (`value` vs `p50/p95/p99` vs `counts/bounds`).
    pub async fn metric_timeseries(
        &self,
        params: &TimeseriesParams,
    ) -> Result<TimeseriesPayload, ApiError> {
        self.get("metrics/timeseries", Some(params)).await
    }

    /// `GET /metrics/attributes?metricName=…&lookbackHours=…` — the keys that
    /// can be used for group-by on this metric.
    pub async fn metric_attributes(
        &self,
        params: &AttributesParams,
    ) -> Result<AttributesResponse, ApiError> {
        self.get("metrics/attributes", Some(params)).await
    }
}

// ----- Descriptors ----------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct MetricListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricDescriptorList {
    pub metrics: Vec<MetricDescriptor>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MetricDescriptor {
    pub metric_name: String,
    pub kind: MetricKind,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    pub last_seen_at: String,
    pub sample_count: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

impl MetricKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

// ----- Timeseries -----------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TimeseriesMode {
    Mean,
    Percentiles,
    Heatmap,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TimeseriesParams {
    pub metric_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<TimeseriesMode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TimeseriesPayload {
    Mean { points: Vec<MetricTimeSeriesPoint> },
    Percentiles { points: Vec<MetricPercentilePoint> },
    Heatmap { points: Vec<MetricHeatmapPoint> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricTimeSeriesPoint {
    pub bucket_ms: i64,
    pub group_key: String,
    pub value: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricPercentilePoint {
    pub bucket_ms: i64,
    pub group_key: String,
    #[serde(default)]
    pub p50: Option<f64>,
    #[serde(default)]
    pub p95: Option<f64>,
    #[serde(default)]
    pub p99: Option<f64>,
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricHeatmapPoint {
    pub bucket_ms: i64,
    pub group_key: String,
    /// `counts.len() == bounds.len() + 1` (the trailing slot is the `+∞`
    /// overflow bucket).
    pub counts: Vec<u64>,
    pub bounds: Vec<f64>,
}

// ----- Attributes -----------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct AttributesParams {
    pub metric_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookback_hours: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttributesResponse {
    pub keys: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metric_kind_serializes_lowercase() {
        let s = serde_json::to_string(&MetricKind::Histogram).unwrap();
        assert_eq!(s, "\"histogram\"");
    }

    #[test]
    fn timeseries_payload_picks_variant_by_field_shape() {
        // Mean
        let mean: TimeseriesPayload = serde_json::from_value(json!({
            "points": [{"bucket_ms": 1, "group_key": "g", "value": 1.0, "count": 1}]
        })).unwrap();
        assert!(matches!(mean, TimeseriesPayload::Mean { .. }));

        // Percentiles (presence of p50/p95/p99 distinguishes)
        let pct: TimeseriesPayload = serde_json::from_value(json!({
            "points": [{"bucket_ms": 1, "group_key": "g", "p50": 1.0, "p95": 2.0, "p99": 3.0, "count": 1}]
        })).unwrap();
        assert!(matches!(pct, TimeseriesPayload::Percentiles { .. }));

        // Heatmap (presence of counts/bounds)
        let hm: TimeseriesPayload = serde_json::from_value(json!({
            "points": [{"bucket_ms": 1, "group_key": "g", "counts": [1, 2, 0], "bounds": [10.0, 50.0]}]
        })).unwrap();
        assert!(matches!(hm, TimeseriesPayload::Heatmap { .. }));
    }

    #[test]
    fn timeseries_params_drops_none_fields() {
        let p = TimeseriesParams {
            metric_name: "http.server.duration".into(),
            since_ms: Some(1_000),
            until_ms: Some(2_000),
            bucket_ms: None,
            group_by: None,
            environment: None,
            service_name: None,
            mode: Some(TimeseriesMode::Mean),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"metric_name\":\"http.server.duration\""));
        assert!(s.contains("\"since_ms\":1000"));
        assert!(s.contains("\"mode\":\"mean\""));
        assert!(!s.contains("\"bucket_ms\""));
        assert!(!s.contains("\"group_by\""));
    }
}
