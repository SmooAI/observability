//! Example: how a downstream Rust service (api-prime, voice, temporal-worker)
//! bootstraps observability and self-emits telemetry.
//!
//! Run with the env vars a real deployment would set:
//!
//! ```bash
//! SMOOAI_OBSERVABILITY_ENDPOINT=https://api.smoo.ai \
//! SMOOAI_OBSERVABILITY_AUTH_URL=https://auth.smoo.ai \
//! SMOOAI_OBSERVABILITY_CLIENT_ID=<m2m-client-id> \
//! SMOOAI_OBSERVABILITY_CLIENT_SECRET=sk_... \
//! SMOOAI_OBSERVABILITY_SERVICE_NAME=smooai-voice \
//! cargo run --example service_bootstrap
//! ```
//!
//! With no env vars set it still runs end-to-end (OTel disabled, capture
//! handler-only) so you can see the shape without a live backend.

use smooai_observability as obs;
use std::fmt;

#[derive(Debug)]
struct DbError {
    detail: String,
}
impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database query failed: {}", self.detail)
    }
}
impl std::error::Error for DbError {}

#[tokio::main]
async fn main() {
    // 1. Bootstrap from environment. Wires the OTel SDK (traces + metrics over
    //    OTLP/HTTP to api.smoo.ai, M2M-authenticated) and a capture client.
    let result = obs::bootstrap().await;
    println!("observability installed: {}", result.installed);

    // Install the bootstrap's client as the process-wide global so the free
    // `obs::capture_*` functions have somewhere to dispatch.
    obs::set_global_client(result.client.clone());

    // 2. Set ambient scope for this task — merged into every captured event.
    obs::set_user(Some(obs::UserContext {
        id: Some("user-123".into()),
        org_id: Some("org-abc".into()),
        session_id: None,
    }));
    obs::set_tag("component", "ingest-worker");
    obs::add_breadcrumb("startup", Some("worker booted".into()), obs::Level::Info);

    // 3. Capture a message and an error (PII in the message is scrubbed).
    obs::capture_message("worker started, token=should-be-redacted", obs::Level::Info);

    let err = DbError {
        detail: "connection reset".into(),
    };
    if let Some(id) = obs::capture_exception(&err) {
        println!("captured exception event id: {id}");
    }

    // 4. Emit application metrics through the OTel meter.
    let metrics = obs::metrics_client("smooai-voice");
    metrics.counter(
        "agent.turn.completed",
        1,
        &[("channel", "voice"), ("tier", "pro")],
    );
    metrics.timing("agent.ttft.ms", 312.0, &[("model", "sonnet")]);

    let result_value: Result<i32, ()> = metrics
        .with_timing(
            "agent.tool.latency.ms",
            &[("tool", "knowledge_search")],
            async {
                // ... do real work ...
                Ok(42)
            },
        )
        .await;
    println!("work result: {result_value:?}");

    // 5. Per-request isolated scope (e.g. one HTTP request / one call).
    obs::with_scope(|scope| async move {
        scope.set_tag("request_id", "req-9999");
        obs::capture_message("handling request", obs::Level::Debug);
    })
    .await;

    // 6. On shutdown, flush traces + metrics + queued error events.
    if let Some(otel) = &result.otel {
        otel.flush();
        otel.shutdown();
    }
    result.client.flush().await;
    println!("flushed; shutting down.");
}
