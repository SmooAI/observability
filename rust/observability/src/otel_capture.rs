//! OTel-native error capture — the Rust port of `go/otel_capture.go`.
//!
//! # Why this file has to exist
//!
//! `bootstrap()` builds the OTLP pipeline and `set_global_client` wires the free
//! `capture_exception` / `capture_message` functions, so on paper a Rust service
//! "can report errors". In practice nothing did: as of 2026-08-12 a grep for
//! `capture_exception` across the SmooAI monorepo's `rust/` tree returned ONE
//! hit, and it was a comment saying the capability was wired.
//!
//! The visible consequence: `error_groups` in production contained fifteen
//! groups, and every one of them came from Node or Lambda — `node:net`,
//! `/var/task/bundle.mjs`, `packages/backend/dist/voice/server.mjs`. Those are
//! the TypeScript services that have since been rewritten in Rust. The error
//! tracker was not quiet because nothing was failing; it was showing a graveyard
//! of services that no longer exist, while the platform's primary language
//! reported nothing at all.
//!
//! Three separate holes made that total rather than partial:
//!   1. no capture handler registered (this file),
//!   2. no panic hook, so a panicking service reported nothing (this file),
//!   3. `otlp.rs` in api-prime has no exception path, so an error SPAN never
//!      becomes an error group either — that one is a server-side change and is
//!      deliberately NOT solved here.
//!
//! # Difference from the Go port
//!
//! Go has no ambient span, so `go/otel_capture.go` needs an explicit
//! `CaptureExceptionOnSpan(ctx, ...)` entry point. Rust does have one —
//! `opentelemetry::Context::current()` follows the `tracing` span through
//! `tracing-opentelemetry` — so the plain `capture_exception` call site
//! correlates automatically and there is no second API to remember.

use std::sync::Arc;

use opentelemetry::trace::{SpanKind, SpanRef, Status, TraceContextExt, Tracer};
use opentelemetry::{global, Context, KeyValue};

use crate::client::Client;
use crate::types::{Level, ObservabilityEvent};

/// Instrumentation scope for synthetic capture spans.
const TRACER_NAME: &str = "smooai.observability";

/// Attach OTel-native capture to `client`, returning the wired clone.
///
/// Every captured exception or message is recorded on the ACTIVE span when there
/// is one, so the error lands on the trace a person is already looking at. When
/// there is no active span — a background task, a panic during startup — a
/// synthetic span is minted instead, because an error nobody can find is the
/// same as no error.
///
/// Fires IN ADDITION to the HTTP transport, matching Go (SMOODEV-1148 parity):
/// the transport feeds `error_groups`, this feeds the trace.
#[must_use]
pub fn register_otel_capture(client: &Client) -> Client {
    client.with_capture_handler(Arc::new(|event: &ObservabilityEvent| {
        record_event(event);
    }))
}

/// Record one event on the current span, or a synthetic one.
fn record_event(event: &ObservabilityEvent) {
    let cx = Context::current();
    let attrs = event_attributes(event);

    if cx.has_active_span() {
        apply(&cx.span(), event, &attrs);
        return;
    }

    // No ambient span. Mint one rather than dropping the error on the floor.
    let name = if event.exception.is_some() {
        "observability.capture_exception"
    } else {
        "observability.capture_message"
    };
    let tracer = global::tracer(TRACER_NAME);
    let synthetic = tracer
        .span_builder(name)
        .with_kind(SpanKind::Internal)
        .start(&tracer);
    // Park it in a Context so the synthetic and ambient paths share one code
    // path: an owned `Span` needs `&mut`, a `SpanRef` has interior mutability,
    // and duplicating `apply` for both is how the two drift apart.
    let synthetic_cx = Context::current().with_span(synthetic);
    apply(&synthetic_cx.span(), event, &attrs);
    synthetic_cx.span().end();
}

fn apply(span: &SpanRef<'_>, event: &ObservabilityEvent, attrs: &[KeyValue]) {
    span.set_attributes(attrs.to_vec());

    match event.exception.as_ref().and_then(|chain| chain.first()) {
        Some(first) => {
            // OTel semconv for exceptions is an EVENT named `exception` with
            // `exception.*` attributes — not a bare span attribute. Backends key
            // their error extraction off this exact shape.
            span.add_event(
                "exception",
                vec![
                    KeyValue::new("exception.type", first.r#type.clone()),
                    KeyValue::new("exception.message", first.value.clone()),
                ],
            );
            span.set_status(Status::error(first.value.clone()));
        }
        None => {
            if let Some(message) = event.message.as_deref() {
                span.add_event(
                    "smoo.message",
                    vec![KeyValue::new("smoo.message", message.to_string())],
                );
                // Only ERROR/FATAL flip the span. An INFO capture_message must
                // not paint a healthy request as failed — the same rule that
                // stops a 404 marking an HTTP span Error.
                if matches!(event.level, Level::Error | Level::Fatal) {
                    span.set_status(Status::error(message.to_string()));
                }
            }
        }
    }
}

fn event_attributes(event: &ObservabilityEvent) -> Vec<KeyValue> {
    let mut attrs = vec![KeyValue::new("smoo.event_id", event.event_id.clone())];
    if let Some(env) = event.environment.as_deref() {
        attrs.push(KeyValue::new(
            "deployment.environment.name",
            env.to_string(),
        ));
    }
    if let Some(release) = event.release.as_deref() {
        attrs.push(KeyValue::new("service.version", release.to_string()));
    }
    attrs.push(KeyValue::new(
        "smoo.level",
        format!("{:?}", event.level).to_lowercase(),
    ));

    if let Some(user) = event.user.as_ref() {
        // Ids only. Never email or name: span attributes are exported to the
        // trace store and rendered in the dashboard.
        if let Some(id) = user.id.as_deref() {
            attrs.push(KeyValue::new("enduser.id", id.to_string()));
        }
        if let Some(org) = user.org_id.as_deref() {
            attrs.push(KeyValue::new("enduser.org_id", org.to_string()));
        }
    }
    if let Some(tags) = event.tags.as_ref() {
        for (key, value) in tags {
            attrs.push(KeyValue::new(format!("smoo.tag.{key}"), value.clone()));
        }
    }
    attrs
}

/// Report panics through the global client, then run the previous hook.
///
/// Without this a Rust service can panic — the single loudest failure it has —
/// and report nothing anywhere. Chaining rather than replacing keeps whatever
/// the process already installed (backtrace printing, abort behaviour) intact;
/// swallowing that would trade one blind spot for another.
///
/// Idempotent is NOT free here: calling this twice chains twice and double-
/// reports. Call it once, from `bootstrap`.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());

        crate::capture_message(format!("panic at {location}: {payload}"), Level::Fatal);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientOptions;
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};

    /// One test, not two, because `global::set_tracer_provider` is process-wide:
    /// two tests each installing a provider race and clobber each other's
    /// exporter, which fails as "nothing was exported" and looks like a bug in
    /// the code under test rather than in the test setup.
    ///
    /// Asserts on exported SpanData, not on the handler having run — a handler
    /// that fires and records nothing is exactly the failure this file fixes.
    #[test]
    fn captures_reach_the_exporter_with_the_right_shape_and_status() {
        let exporter = InMemorySpanExporterBuilder::new().build();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        global::set_tracer_provider(provider.clone());

        let client = register_otel_capture(&Client::init(ClientOptions::default()));

        let err = std::io::Error::other("disk went away");
        client.capture_exception(&err);
        client.capture_message("just fyi", Level::Info);

        provider.force_flush().ok();
        let spans = exporter.get_finished_spans().expect("exporter readable");
        assert_eq!(spans.len(), 2, "one synthetic span per capture");

        let exception_span = spans
            .iter()
            .find(|s| s.name == "observability.capture_exception")
            .expect("exception capture exported a span");
        let event = exception_span
            .events
            .iter()
            .find(|e| e.name == "exception")
            .expect(
                "an `exception` EVENT — the semconv shape backends key off, not a bare attribute",
            );
        assert!(event
            .attributes
            .iter()
            .any(|kv| kv.key.as_str() == "exception.message"));
        assert!(
            matches!(exception_span.status, Status::Error { .. }),
            "a captured exception must mark the span Error"
        );

        // The other half of the rule: an INFO message must NOT paint the span
        // failed, the same way a 404 must not mark an HTTP span Error.
        let message_span = spans
            .iter()
            .find(|s| s.name == "observability.capture_message")
            .expect("message capture exported a span");
        assert!(
            !matches!(message_span.status, Status::Error { .. }),
            "an INFO capture_message must not mark the span Error"
        );
        assert!(message_span.events.iter().any(|e| e.name == "smoo.message"));
    }
}
