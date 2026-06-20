//! Application-metrics client — a thin SmooAI-flavored wrapper over the global
//! OpenTelemetry meter, mirroring the TS `metrics/index.ts` surface.
//!
//! ```no_run
//! use smooai_observability::metrics::metrics_client;
//! let m = metrics_client("smooai-voice");
//! m.counter("agent.turn.completed", 1, &[("channel", "voice")]);
//! m.timing("agent.ttft.ms", 312.0, &[("model", "sonnet")]);
//! let stop = m.start_timer("agent.tool.latency.ms", &[("tool", "knowledge")]);
//! // ... do work ...
//! stop();
//! ```
//!
//! Instruments are cached by `(meter_name, instrument_name[, unit])` so we don't
//! leak Meter handles across repeated calls. Reads route through the OTel global
//! meter provider, which [`crate::otel::setup_otel_sdk`] installs.

use once_cell::sync::Lazy;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::KeyValue;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

type AttrSlice<'a> = &'a [(&'a str, &'a str)];

fn to_kvs(attrs: AttrSlice) -> Vec<KeyValue> {
    attrs
        .iter()
        .map(|(k, v)| KeyValue::new(k.to_string(), v.to_string()))
        .collect()
}

static COUNTER_CACHE: Lazy<Mutex<HashMap<String, Counter<u64>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static HISTOGRAM_CACHE: Lazy<Mutex<HashMap<String, Histogram<f64>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn get_counter(meter_name: &str, name: &str) -> Option<Counter<u64>> {
    let key = format!("{meter_name}::{name}");
    let mut cache = COUNTER_CACHE.lock().ok()?;
    if let Some(c) = cache.get(&key) {
        return Some(c.clone());
    }
    let scope = opentelemetry::InstrumentationScope::builder(meter_name.to_string()).build();
    let meter = opentelemetry::global::meter_provider().meter_with_scope(scope);
    let counter = meter.u64_counter(name.to_string()).build();
    cache.insert(key, counter.clone());
    Some(counter)
}

fn get_histogram(meter_name: &str, name: &str, unit: Option<&str>) -> Option<Histogram<f64>> {
    let key = format!("{meter_name}::{name}::{}", unit.unwrap_or(""));
    let mut cache = HISTOGRAM_CACHE.lock().ok()?;
    if let Some(h) = cache.get(&key) {
        return Some(h.clone());
    }
    let scope = opentelemetry::InstrumentationScope::builder(meter_name.to_string()).build();
    let meter = opentelemetry::global::meter_provider().meter_with_scope(scope);
    let mut builder = meter.f64_histogram(name.to_string());
    if let Some(u) = unit {
        builder = builder.with_unit(u.to_string());
    }
    let hist = builder.build();
    cache.insert(key, hist.clone());
    Some(hist)
}

/// A metrics client bound to a specific service-named meter. Cheap to create;
/// call per service / module if you want logical grouping. Defaults to meter
/// name `@smooai/observability` via [`metrics_client_default`].
#[derive(Clone)]
pub struct MetricsClient {
    meter_name: String,
}

impl MetricsClient {
    /// Add to a monotonically-increasing counter. Never panics — a missing
    /// provider just no-ops.
    pub fn counter(&self, name: &str, value: u64, attrs: AttrSlice) {
        if let Some(c) = get_counter(&self.meter_name, name) {
            c.add(value, &to_kvs(attrs));
        }
    }

    /// Record a histogram observation (latencies, sizes, …).
    pub fn histogram(&self, name: &str, value: f64, attrs: AttrSlice) {
        if let Some(h) = get_histogram(&self.meter_name, name, None) {
            h.record(value, &to_kvs(attrs));
        }
    }

    /// Alias for `histogram` with `unit: "ms"` baked in.
    pub fn timing(&self, name: &str, ms: f64, attrs: AttrSlice) {
        if let Some(h) = get_histogram(&self.meter_name, name, Some("ms")) {
            h.record(ms, &to_kvs(attrs));
        }
    }

    /// Start a wall-clock timer. Call the returned closure to record elapsed ms
    /// as a timing histogram. Owns its attributes so the closure is `'static`.
    pub fn start_timer(&self, name: &str, attrs: AttrSlice) -> impl FnOnce() {
        let start = Instant::now();
        let meter_name = self.meter_name.clone();
        let name = name.to_string();
        let owned: Vec<KeyValue> = to_kvs(attrs);
        move || {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if let Some(h) = get_histogram(&meter_name, &name, Some("ms")) {
                h.record(ms, &owned);
            }
        }
    }

    /// Wrap an async future in a timing measurement. Records elapsed ms with a
    /// `status=success|error` attribute. `E` is the future's error type.
    pub async fn with_timing<F, T, E>(
        &self,
        name: &str,
        attrs: AttrSlice<'_>,
        fut: F,
    ) -> Result<T, E>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        let start = Instant::now();
        let result = fut.await;
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let status = if result.is_ok() { "success" } else { "error" };
        let mut kvs = to_kvs(attrs);
        kvs.push(KeyValue::new("status", status));
        if let Some(h) = get_histogram(&self.meter_name, name, Some("ms")) {
            h.record(ms, &kvs);
        }
        result
    }
}

/// Build a metrics client bound to `meter_name` (e.g. `smooai-voice`).
pub fn metrics_client(meter_name: impl Into<String>) -> MetricsClient {
    MetricsClient {
        meter_name: meter_name.into(),
    }
}

/// Default-named metrics client (`@smooai/observability`).
pub fn metrics_client_default() -> MetricsClient {
    metrics_client("@smooai/observability")
}

/// Test seam — drop cached instruments so a fresh MeterProvider takes effect.
pub fn reset_instrument_cache_for_tests() {
    if let Ok(mut c) = COUNTER_CACHE.lock() {
        c.clear();
    }
    if let Ok(mut h) = HISTOGRAM_CACHE.lock() {
        h.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_no_provider_does_not_panic() {
        // With no MeterProvider installed, the global no-op meter is used.
        let m = metrics_client("test-meter");
        m.counter("x.count", 1, &[("a", "b")]);
        m.histogram("x.size", 12.0, &[]);
        m.timing("x.ms", 5.0, &[("k", "v")]);
        let stop = m.start_timer("x.timer.ms", &[("k", "v")]);
        stop();
    }

    #[tokio::test]
    async fn with_timing_propagates_result() {
        let m = metrics_client("test-meter");
        let ok: Result<i32, ()> = m.with_timing("x.op.ms", &[], async { Ok(42) }).await;
        assert_eq!(ok.unwrap(), 42);
        let err: Result<i32, &str> = m.with_timing("x.op.ms", &[], async { Err("boom") }).await;
        assert_eq!(err.unwrap_err(), "boom");
    }
}
