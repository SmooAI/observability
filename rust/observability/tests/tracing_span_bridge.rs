#![cfg(feature = "tracing-bridge")]
//! `tracing` spans must actually reach the exporter — the gap that made a
//! production LLM-tracing outage invisible (th-eaccd1).
//!
//! `bootstrap()` sets the global tracer provider, so anything opened through the
//! OpenTelemetry API exports. `tracing` spans (`#[instrument]`, `info_span!`)
//! do NOT: they need a `tracing-opentelemetry` layer inside the installed
//! subscriber. Without one they are printed and dropped, while every other
//! signal reports the service as instrumented.
//!
//! These assert on spans that arrive at a collector, not on the layer being
//! constructible — a layer that builds and exports nothing is precisely the bug.

use std::sync::{Arc, Mutex};

use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};

/// Collects exported spans so the test can assert on what actually left.
#[derive(Clone, Default, Debug)]
struct CollectingExporter {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl SpanExporter for CollectingExporter {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send {
        self.spans.lock().unwrap().extend(batch);
        async { Ok(()) }
    }
}

fn provider_with(exporter: CollectingExporter) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_simple_exporter(exporter)
        .build()
}

/// The headline case: a `tracing` span, opened the way application code opens
/// them, must arrive at the exporter with its name intact.
///
/// Remove the layer from the registry and this fails with zero spans — which is
/// exactly what production looked like.
#[test]
fn a_tracing_span_reaches_the_exporter() {
    use opentelemetry::trace::TracerProvider as _;
    use tracing_subscriber::prelude::*;

    let exporter = CollectingExporter::default();
    let provider = provider_with(exporter.clone());
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer("smooai-observability/tracing"));

    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("gen_ai.chat", gen_ai.request.model = "test-model");
        let _entered = span.enter();
    });

    provider.force_flush().ok();
    let spans = exporter.spans.lock().unwrap();
    assert!(
        spans.iter().any(|s| s.name == "gen_ai.chat"),
        "a tracing span did not reach the exporter — exported names: {:?}",
        spans.iter().map(|s| s.name.as_ref()).collect::<Vec<_>>()
    );
}

/// The negative control, and the reason the first test means anything: the SAME
/// span with NO bridge layer must export NOTHING.
///
/// This is the production configuration that looked healthy — provider
/// installed, spans opened, nothing exported.
#[test]
fn without_the_bridge_layer_a_tracing_span_exports_nothing() {
    let exporter = CollectingExporter::default();
    let provider = provider_with(exporter.clone());

    // A subscriber with NO otel layer — what a host gets from a plain
    // fmt-only `init_telemetry()`.
    let subscriber = tracing_subscriber::registry();
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("gen_ai.chat");
        let _entered = span.enter();
    });

    provider.force_flush().ok();
    assert!(
        exporter.spans.lock().unwrap().is_empty(),
        "spans exported without a bridge layer — then the bridge is not what makes them export, \
         and the first test proves nothing"
    );
}

/// Span ATTRIBUTES must survive the bridge. A span that exports its name but
/// drops `gen_ai.request.model` / token usage is useless for LLM tracing — the
/// attributes are the product.
#[test]
fn span_attributes_survive_the_bridge() {
    use opentelemetry::trace::TracerProvider as _;
    use tracing_subscriber::prelude::*;

    let exporter = CollectingExporter::default();
    let provider = provider_with(exporter.clone());
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer("smooai-observability/tracing"));

    tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
        let span = tracing::info_span!(
            "gen_ai.chat",
            gen_ai.request.model = "groq-gpt-oss-120b",
            gen_ai.usage.input_tokens = 42_i64
        );
        let _entered = span.enter();
    });

    provider.force_flush().ok();
    let spans = exporter.spans.lock().unwrap();
    let chat = spans
        .iter()
        .find(|s| s.name == "gen_ai.chat")
        .expect("gen_ai.chat span exported");
    let keys: Vec<String> = chat
        .attributes
        .iter()
        .map(|kv| kv.key.as_str().to_string())
        .collect();
    assert!(
        keys.iter().any(|k| k == "gen_ai.request.model"),
        "model attribute lost crossing the bridge; got {keys:?}"
    );
    assert!(
        keys.iter().any(|k| k == "gen_ai.usage.input_tokens"),
        "token usage lost crossing the bridge; got {keys:?}"
    );
}
