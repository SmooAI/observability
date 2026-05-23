//! Metrics view types. Mirror of `packages/backend/src/routes/observability/metrics-query.ts`.
//!
//! Filled in during phase 5 of SMOODEV-1175.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize)]
pub struct TimeseriesParams {
    pub metric_name: String,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub bucket_ms: Option<i64>,
    pub group_by: Option<String>,
    pub environment: Option<String>,
    pub service_name: Option<String>,
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
