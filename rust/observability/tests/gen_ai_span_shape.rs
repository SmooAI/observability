//! The two GenAI cross-SDK divergences the README's honest-divergence ledger
//! used to record, now closed and pinned.
//!
//! Both need a real exported span, not a string, so they live here rather than
//! in `gen_ai.rs`'s unit tests.

use opentelemetry::trace::{Tracer, TracerProvider as _};
use opentelemetry::{Array, Value};
use opentelemetry_sdk::trace::{in_memory_exporter::InMemorySpanExporter, SdkTracerProvider};
use smooai_observability::gen_ai::{
    record_gen_ai_message, set_gen_ai_attributes, GenAIAttributes, GenAIMessageExtra, GenAIRole,
};

fn recorder() -> (SdkTracerProvider, InMemorySpanExporter) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    (provider, exporter)
}

/// `gen_ai.tool.names` is a STRING ARRAY, as in the TS/Python/Go/.NET SDKs.
///
/// It used to be `names.join(",")` here and only here, so a Rust service's
/// spans could not be filtered by tool the way every other language's could —
/// and a tool name containing a comma silently became two tools.
#[test]
fn tool_names_is_a_string_array() {
    let (provider, exporter) = recorder();
    let mut span = provider.tracer("t").start("llm");
    set_gen_ai_attributes(
        &mut span,
        &GenAIAttributes {
            tool_names: Some(vec!["search".into(), "calc".into()]),
            ..Default::default()
        },
    );
    drop(span);
    provider.force_flush().expect("flush");

    let spans = exporter.get_finished_spans().expect("spans");
    let kv = spans[0]
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "gen_ai.tool.names")
        .expect("gen_ai.tool.names not set");
    match &kv.value {
        Value::Array(Array::String(values)) => {
            let got: Vec<&str> = values.iter().map(|v| v.as_str()).collect();
            assert_eq!(got, ["search", "calc"]);
        }
        other => panic!("gen_ai.tool.names is {other:?}, want a string array"),
    }
}

/// An empty tool list emits nothing at all — same as the other four SDKs.
#[test]
fn empty_tool_names_emits_no_attribute() {
    let (provider, exporter) = recorder();
    let mut span = provider.tracer("t").start("llm");
    set_gen_ai_attributes(
        &mut span,
        &GenAIAttributes {
            tool_names: Some(vec![]),
            ..Default::default()
        },
    );
    drop(span);
    provider.force_flush().expect("flush");

    let spans = exporter.get_finished_spans().expect("spans");
    assert!(!spans[0]
        .attributes
        .iter()
        .any(|kv| kv.key.as_str() == "gen_ai.tool.names"));
}

/// Recorded message content is PII-scrubbed. Prompts and tool arguments are the
/// most PII-dense payload this SDK touches, and only the TS SDK used to scrub
/// them.
#[test]
fn recorded_message_content_is_scrubbed() {
    let (provider, exporter) = recorder();
    let mut span = provider.tracer("t").start("llm");
    record_gen_ai_message(
        &mut span,
        GenAIRole::User,
        "mail a@b.com, Authorization: Bearer abc.def-ghi",
        &GenAIMessageExtra::default(),
    );
    drop(span);
    provider.force_flush().expect("flush");

    let spans = exporter.get_finished_spans().expect("spans");
    let event = &spans[0].events[0];
    let content = event
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "gen_ai.message.content")
        .map(|kv| kv.value.as_str().to_string())
        .expect("content attribute");

    assert!(
        !content.contains("a@b.com"),
        "raw email survived: {content}"
    );
    assert!(
        content.contains("Bearer [redacted]"),
        "bearer token not redacted: {content}"
    );
    // No key is installed in this test process, so the email is redacted rather
    // than hashed — the fail-safe, not a missed match.
    assert!(
        content.contains("[email:"),
        "email was not replaced by a token: {content}"
    );
}
