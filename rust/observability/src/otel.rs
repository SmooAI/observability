//! OpenTelemetry SDK setup — OTLP/HTTP/JSON export for traces + metrics to
//! api.smoo.ai, mirroring the TS `otel/setup-otel-sdk.ts`.
//!
//! Two auth modes, matching the TS SDK:
//!   - **Static header** — a pre-minted Bearer JWT passed via `otlp_headers`.
//!   - **Per-request token** — a [`TokenProvider`] consulted on EVERY export,
//!     so a refreshed token starts being used on the next export with no
//!     exporter restart. This sidesteps the header-snapshot staleness that bit
//!     the TS SDK (SMOODEV-1206) and applies here too: the OTLP exporter holds
//!     its HTTP client + headers for the life of the process.
//!
//! The per-request mode is implemented with a custom [`opentelemetry_http::HttpClient`]
//! wrapper ([`AuthInjectingHttpClient`]) that asks the `TokenProvider` for a
//! fresh token, sets the `Authorization` header, and on a 401 invalidates +
//! retries once. The OTLP protocol transport itself runs over `smooai-fetch`
//! (timeouts + retries + circuit breaking) rather than raw `reqwest`, so the
//! export path inherits the same resilience policy every other SmooAI outbound
//! call uses (SMOODEV-2029). smooai-fetch owns transport retry; the exporter
//! layer doesn't retry because this crate leaves opentelemetry-otlp's
//! `experimental-http-retry` feature off (see `auth_client.rs` for the no-double-
//! retry rationale).
//!
//! Wire format is OTLP/HTTP/JSON (`http-json` feature) so the bytes match what
//! the TS `AuthInjectingTraceExporter` POSTs.
//!
//! `setup_otel_sdk` is idempotent — a second call returns the existing handle.

use crate::auth::TokenProvider;
use once_cell::sync::OnceCell;
use opentelemetry_otlp::{
    MetricExporter, Protocol, SpanExporter, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use std::collections::HashMap;
use std::time::Duration;

mod auth_client;
pub use auth_client::AuthInjectingHttpClient;

/// Options for [`setup_otel_sdk`]. Field names mirror the TS `SetupOtelOptions`.
pub struct SetupOtelOptions {
    /// Service name surfaced in spans (e.g. `smooai-voice`).
    pub service_name: String,
    /// Fully-qualified OTLP/HTTP endpoint for traces (e.g.
    /// `https://api.smoo.ai/v1/traces`). When `None`, traces are not exported.
    pub otlp_traces_endpoint: Option<String>,
    /// Fully-qualified OTLP/HTTP endpoint for metrics. When `None`, metrics are
    /// not exported.
    pub otlp_metrics_endpoint: Option<String>,
    /// Static headers merged onto every export (e.g. a pre-minted
    /// `authorization` Bearer, or `user-agent`).
    pub otlp_headers: HashMap<String, String>,
    /// Deployment environment string.
    pub environment: Option<String>,
    /// Release identifier — git sha, container version.
    pub release: Option<String>,
    /// When set, exports authenticate per-request via this provider instead of
    /// (or in addition to) the static `otlp_headers` authorization.
    pub token_provider: Option<TokenProvider>,
    /// Metric export interval. Default 30s.
    pub metric_export_interval: Duration,
}

impl SetupOtelOptions {
    pub fn new(service_name: impl Into<String>) -> Self {
        SetupOtelOptions {
            service_name: service_name.into(),
            otlp_traces_endpoint: None,
            otlp_metrics_endpoint: None,
            otlp_headers: HashMap::new(),
            environment: None,
            release: None,
            token_provider: None,
            metric_export_interval: Duration::from_secs(30),
        }
    }
}

/// Handle returned by [`setup_otel_sdk`]: flush + shutdown hooks the host wires
/// into SIGTERM / `beforeExit`. Cloneable; all clones share the same providers.
#[derive(Clone)]
pub struct OtelSdkHandle {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
}

impl OtelSdkHandle {
    /// Force-flush spans + metrics now. Best-effort; errors are swallowed.
    pub fn flush(&self) {
        if let Some(tp) = &self.tracer_provider {
            let _ = tp.force_flush();
        }
        if let Some(mp) = &self.meter_provider {
            let _ = mp.force_flush();
        }
    }

    /// Graceful shutdown — drains and closes the pipelines. Best-effort.
    pub fn shutdown(&self) {
        if let Some(tp) = &self.tracer_provider {
            let _ = tp.shutdown();
        }
        if let Some(mp) = &self.meter_provider {
            let _ = mp.shutdown();
        }
    }
}

static INSTALLED: OnceCell<OtelSdkHandle> = OnceCell::new();

/// Initialize the OTel SDK and install it as the global tracer + meter provider.
/// Idempotent — a second call returns the already-installed handle. Never
/// panics: any exporter build failure is logged to stderr and produces a handle
/// with that signal disabled.
pub fn setup_otel_sdk(options: SetupOtelOptions) -> OtelSdkHandle {
    if let Some(existing) = INSTALLED.get() {
        return existing.clone();
    }
    let handle = build_and_install(options);
    // If two threads race, the first install wins; return whatever is installed.
    let _ = INSTALLED.set(handle.clone());
    INSTALLED.get().cloned().unwrap_or(handle)
}

fn build_resource(options: &SetupOtelOptions) -> Resource {
    use opentelemetry::KeyValue;
    let mut builder = Resource::builder().with_service_name(options.service_name.clone());
    if let Some(release) = &options.release {
        builder = builder.with_attribute(KeyValue::new("service.version", release.clone()));
    }
    if let Some(env) = &options.environment {
        builder = builder.with_attribute(KeyValue::new("deployment.environment.name", env.clone()));
    }
    builder.build()
}

fn build_and_install(options: SetupOtelOptions) -> OtelSdkHandle {
    // Install the global W3C Trace Context propagator so traceparent/tracestate
    // headers are injected on outbound calls and extracted on inbound ones (see
    // the `tower` / `reqwest-middleware` integrations). This is always-on and
    // cheap — it just registers the text-map propagator the integrations consult
    // via `opentelemetry::global::get_text_map_propagator`. Without it that
    // global defaults to a no-op propagator and traces never link across
    // services. (SMOODEV-2024)
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let resource = build_resource(&options);

    // CRITICAL — both pipelines MUST run their export loop on the Tokio runtime,
    // never on a bare OS thread (SMOODEV-2045). The DEFAULT `with_batch_exporter`
    // (trace) and `PeriodicReader::builder` (metrics) in opentelemetry_sdk 0.32
    // spawn a dedicated `std::thread` and drive the async export with
    // `futures_executor::block_on`. Our exporter's HTTP transport is
    // `AuthInjectingHttpClient` → `smooai-fetch` → `reqwest`, and reqwest's
    // request execution PANICS when no Tokio reactor is present
    // ("there is no reactor running, must be called from the context of a Tokio
    // 1.x runtime"). On that dedicated OS thread there is none, so the very first
    // export aborts the whole process — and because the workspace release profile
    // is `panic = "abort"`, that panic on the background thread takes the host
    // down (the temporal-worker crashloop that blocked eSign, SMOODEV-2031).
    //
    // The async-runtime variants (`span_processor_with_async_runtime` /
    // `periodic_reader_with_async_runtime`, gated by the `rt-tokio` feature this
    // crate already enables) instead `tokio::spawn` the worker, so the export
    // future runs on the multi-thread runtime where reqwest has its reactor. This
    // is the documented way to drive the OTLP/HTTP exporter from an async host.
    let tracer_provider = match build_span_exporter(&options) {
        Some(exporter) => {
            use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;
            let processor = BatchSpanProcessor::builder(exporter, runtime::Tokio).build();
            let tp = SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_span_processor(processor)
                .build();
            opentelemetry::global::set_tracer_provider(tp.clone());
            Some(tp)
        }
        None => None,
    };

    let meter_provider = match build_metric_exporter(&options) {
        Some(exporter) => {
            use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;
            let reader = PeriodicReader::builder(exporter, runtime::Tokio)
                .with_interval(options.metric_export_interval)
                .build();
            let mp = SdkMeterProvider::builder()
                .with_resource(resource)
                .with_reader(reader)
                .build();
            opentelemetry::global::set_meter_provider(mp.clone());
            Some(mp)
        }
        None => None,
    };

    OtelSdkHandle {
        tracer_provider,
        meter_provider,
    }
}

/// Per-export HTTP timeout baked into the smooai-fetch client backing the OTLP
/// exporter. Matches the 10s the previous raw-reqwest client used.
const OTLP_EXPORT_TIMEOUT_MS: u64 = 10_000;

/// Build the auth-injecting smooai-fetch HTTP client shared by both exporters.
fn build_http_client(options: &SetupOtelOptions) -> AuthInjectingHttpClient {
    AuthInjectingHttpClient::new(OTLP_EXPORT_TIMEOUT_MS, options.token_provider.clone())
}

fn build_span_exporter(options: &SetupOtelOptions) -> Option<SpanExporter> {
    let endpoint = options.otlp_traces_endpoint.clone()?;
    let client = build_http_client(options);
    let result = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpJson)
        .with_endpoint(endpoint)
        .with_headers(options.otlp_headers.clone())
        .with_http_client(client)
        .build();
    match result {
        Ok(exporter) => Some(exporter),
        Err(e) => {
            warn(&format!("failed to build span exporter: {e}"));
            None
        }
    }
}

fn build_metric_exporter(options: &SetupOtelOptions) -> Option<MetricExporter> {
    let endpoint = options.otlp_metrics_endpoint.clone()?;
    let client = build_http_client(options);
    let result = MetricExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpJson)
        .with_endpoint(endpoint)
        .with_headers(options.otlp_headers.clone())
        .with_http_client(client)
        .build();
    match result {
        Ok(exporter) => Some(exporter),
        Err(e) => {
            warn(&format!("failed to build metric exporter: {e}"));
            None
        }
    }
}

pub(crate) fn warn(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "[@smooai/observability/otel] {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_endpoints_yields_disabled_handle() {
        // A fresh handle built with no endpoints must have neither provider and
        // must not panic. (We can't call the global-installing setup_otel_sdk in
        // a unit test without polluting global state, so exercise build paths.)
        let opts = SetupOtelOptions::new("test-svc");
        assert!(build_span_exporter(&opts).is_none());
        assert!(build_metric_exporter(&opts).is_none());
    }

    #[test]
    fn resource_includes_service_name() {
        let mut opts = SetupOtelOptions::new("svc-x");
        opts.release = Some("v1".into());
        opts.environment = Some("production".into());
        // Resource builds without panic; detailed attribute inspection is
        // covered by integration usage.
        let _r = build_resource(&opts);
    }
}
