//! Logs-signal integration tests (th-5dca7d / th-de3805).
//!
//! The whole point of the logs signal is trace↔log correlation: a log emitted
//! inside an active span must carry that span's real W3C `trace_id`/`span_id`,
//! so the observability product can join logs to traces. This test drives that
//! path deterministically with an in-memory exporter.
//!
//! It uses the exact bridge `OtelSdkHandle::tracing_appender_layer()` hands the
//! host — `OpenTelemetryTracingBridge::new(&logger_provider)` — and a
//! `simple` log processor (synchronous export on emit) so the assertion has no
//! flush-timing / runtime dependency. The correlation itself is the SDK logger's
//! doing: it reads `opentelemetry::Context::current()` at emit time, which is the
//! same OTel-native context our tower/gen_ai/reqwest spans make active.

use opentelemetry::logs::{AnyValue, Severity};
use opentelemetry::trace::{TraceContextExt, Tracer, TracerProvider};
use opentelemetry::Context;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::info;
use tracing_subscriber::prelude::*;

#[test]
fn log_within_active_span_carries_trace_and_span_id() {
    // Logs pipeline: in-memory exporter behind a simple (synchronous) processor.
    let exporter = InMemoryLogExporter::default();
    let logger_provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();

    // The bridge the host installs — identical to `tracing_appender_layer()`.
    let bridge = OpenTelemetryTracingBridge::new(&logger_provider);
    let subscriber = tracing_subscriber::registry().with(bridge);

    // A real sampled OTel span so its SpanContext carries valid W3C ids.
    let tracer_provider = SdkTracerProvider::builder().build();
    let tracer = tracer_provider.tracer("logs-signal-test");
    let cx = Context::current().with_span(tracer.start("unit-of-work"));
    let span_ctx = cx.span().span_context().clone();
    assert!(
        span_ctx.is_valid(),
        "test span must have a valid (sampled) span context"
    );

    tracing::subscriber::with_default(subscriber, || {
        // Make the span the current OTel context, then log inside it.
        let _guard = cx.clone().attach();
        info!("hello from within a span");
    });

    logger_provider.force_flush().unwrap();

    let logs = exporter.get_emitted_logs().expect("emitted logs readable");
    assert_eq!(logs.len(), 1, "exactly one record should have been emitted");
    let record = &logs[0].record;

    // Body ← message.
    match record.body() {
        Some(AnyValue::String(s)) => assert_eq!(s.as_str(), "hello from within a span"),
        other => panic!("unexpected log body: {other:?}"),
    }

    // Severity ← tracing level.
    assert_eq!(record.severity_number(), Some(Severity::Info));

    // trace_id / span_id ← ACTIVE span. This is the correlation the product needs.
    let trace_context = record
        .trace_context()
        .expect("record emitted inside a span must carry trace context");
    assert_eq!(
        trace_context.trace_id,
        span_ctx.trace_id(),
        "log record trace_id must match the active span's trace_id"
    );
    assert_eq!(
        trace_context.span_id,
        span_ctx.span_id(),
        "log record span_id must match the active span's span_id"
    );
}

#[test]
fn log_without_active_span_has_no_trace_context() {
    // Outside any span the record still flows through the pipeline, but carries
    // no trace context — nothing to correlate to, and nothing fabricated.
    let exporter = InMemoryLogExporter::default();
    let logger_provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let bridge = OpenTelemetryTracingBridge::new(&logger_provider);
    let subscriber = tracing_subscriber::registry().with(bridge);

    tracing::subscriber::with_default(subscriber, || {
        info!("no span here");
    });

    logger_provider.force_flush().unwrap();
    let logs = exporter.get_emitted_logs().unwrap();
    assert_eq!(logs.len(), 1);
    assert!(
        logs[0].record.trace_context().is_none(),
        "a log with no active span must not carry a (fabricated) trace context"
    );
}
