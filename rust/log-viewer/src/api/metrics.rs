//! Metrics view types. Mirror of `packages/backend/src/routes/observability/metrics-query.ts`.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use super::{ApiError, OrgClient};

impl<'a> OrgClient<'a> {
    pub async fn list_metrics(
        &self,
        params: &MetricListParams,
    ) -> Result<MetricDescriptorList, ApiError> {
        self.get("metrics", Some(params)).await
    }

    pub async fn metric_timeseries(
        &self,
        params: &TimeseriesParams,
    ) -> Result<TimeseriesPayload, ApiError> {
        self.get("metrics/timeseries", Some(params)).await
    }

    pub async fn metric_attributes(
        &self,
        params: &AttributesParams,
    ) -> Result<AttributesResponse, ApiError> {
        self.get("metrics/attributes", Some(params)).await
    }
}

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TimeseriesMode {
    Mean,
    Percentiles,
    Heatmap,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricDescriptor {
    pub metric_name: String,
    pub kind: MetricKind,
    pub unit: Option<String>,
    pub service_name: Option<String>,
    pub last_seen_at: String,
    pub sample_count: u64,
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
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricHeatmapPoint {
    pub bucket_ms: i64,
    pub group_key: String,
    pub counts: Vec<u64>,
    pub bounds: Vec<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TimeseriesPayload {
    Mean { points: Vec<MetricTimeSeriesPoint> },
    Percentiles { points: Vec<MetricPercentilePoint> },
    Heatmap { points: Vec<MetricHeatmapPoint> },
}
