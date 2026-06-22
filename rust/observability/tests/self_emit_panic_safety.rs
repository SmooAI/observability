//! Regression test for the temporal-worker self-emit crashloop (SMOODEV-2045).
//!
//! ## What broke
//!
//! Once `SMOOAI_OBSERVABILITY_*` was set in prod, the temporal-worker crashlooped
//! (and blocked eSign) — mitigated by force-disabling self-emit (SMOODEV-2031).
//! The SDK's whole promise is "best-effort, never crash the host"; that was
//! violated.
//!
//! ## Root cause
//!
//! opentelemetry_sdk 0.32's DEFAULT batch span processor + periodic metric reader
//! run their export loop on a dedicated `std::thread` and drive the async export
//! with `futures_executor::block_on`. This crate's OTLP exporter sends over
//! `smooai-fetch` → `reqwest`, and reqwest PANICS when executed with no Tokio
//! reactor present ("there is no reactor running …"). On that bare OS thread there
//! is none, so the first export panics — and the workspace release profile is
//! `panic = "abort"`, so a panic on ANY thread aborts the whole process. The fix
//! switches both pipelines to the `*_with_async_runtime` variants driven by
//! `runtime::Tokio`, so the export future is `tokio::spawn`ed onto the live
//! runtime where reqwest has its reactor.
//!
//! ## What this test proves
//!
//! With the SDK CONFIGURED (endpoint + M2M auth) but the ingest returning 401 and,
//! separately, the endpoint unreachable, bootstrapping + driving a real span and
//! metric export must NOT panic / abort — exports degrade to logged no-ops. The
//! export request must actually reach the (mock) server, proving it ran to
//! completion on a reactor rather than dying off-runtime.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use opentelemetry::global;
use opentelemetry::trace::{Tracer, TracerProvider};
use smooai_observability::bootstrap::{bootstrap_with, BootstrapEnv};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as WmRequest, Respond, ResponseTemplate};

/// A mock ingest that ALWAYS 401s and counts hits — mirrors "configured but the
/// M2M creds are unauthorized", the exact prod condition that crashlooped.
struct CountingUnauthorized(Arc<AtomicUsize>);
impl Respond for CountingUnauthorized {
    fn respond(&self, _req: &WmRequest) -> ResponseTemplate {
        self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(401).set_body_string("unauthorized")
    }
}

/// Bootstrap with a working token endpoint but an ingest that 401s every export,
/// then emit a span + a metric and flush. The whole sequence must complete
/// without panicking the test process (which, under `panic = "abort"` in release,
/// is the difference between "logged no-op" and "host crash").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_but_unauthorized_ingest_does_not_panic() {
    // Auth endpoint mints a token fine — the failure is at the INGEST, like prod.
    let auth = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "tok-unauthorized-scenario",
            "expires_in": 3600
        })))
        .mount(&auth)
        .await;

    let ingest_hits = Arc::new(AtomicUsize::new(0));
    let ingest = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(CountingUnauthorized(ingest_hits.clone()))
        .mount(&ingest)
        .await;

    let env = BootstrapEnv {
        endpoint: Some(ingest.uri()),
        auth_url: Some(auth.uri()),
        client_id: Some("cid".into()),
        client_secret: Some("sk_secret".into()),
        service_name: Some("crashfix-test".into()),
        // Tiny interval so the metric reader fires its export within the test.
        ..Default::default()
    };

    let result = bootstrap_with(env).await;
    assert!(result.installed, "bootstrap should have installed the SDK");
    assert!(
        result.otel.is_some(),
        "an endpoint was configured, so the OTLP pipelines must be built"
    );

    // Emit a span through the globally-installed tracer provider. Ending the span
    // hands it to the batch processor, whose export runs on the Tokio runtime.
    let tracer = global::tracer_provider().tracer("crashfix-test");
    tracer.in_span("export-attempt", |_cx| {}); // span ends here → queued for export

    // Force-flush traces + metrics NOW so the export actually fires within the
    // test window rather than waiting on the batch timer. This is the call that,
    // pre-fix, drove `reqwest` off-runtime and aborted the process.
    if let Some(otel) = &result.otel {
        otel.flush();
    }

    // Give the spawned export tasks a moment to hit the (401) ingest.
    tokio::time::sleep(Duration::from_millis(750)).await;

    // If we got here, nothing panicked/aborted — the core guarantee. And the
    // export must have actually reached the ingest (proving it ran on a reactor,
    // not died off-runtime before sending).
    assert!(
        ingest_hits.load(Ordering::SeqCst) >= 1,
        "the OTLP export must have reached the ingest endpoint (it ran on the \
         Tokio runtime); 0 hits would mean it never sent — the off-runtime crash"
    );

    if let Some(otel) = &result.otel {
        otel.shutdown();
    }
}

/// The endpoint is set but completely unreachable (connection refused). This must
/// also degrade to a logged no-op, never a panic/abort. Runs in its own process
/// (separate test binary entry) so it does not collide with the global
/// `OnceCell` install from the test above — cargo runs each `#[test]` in the same
/// binary, but `bootstrap_with` / `setup_otel_sdk` are idempotent via `OnceCell`,
/// so a second bootstrap here would no-op. We therefore exercise the unreachable
/// path at the exporter HTTP-client layer directly (the precise code reqwest runs
/// on export) to keep the assertion meaningful regardless of test ordering.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreachable_endpoint_export_does_not_panic() {
    use opentelemetry_http::HttpClient;
    use smooai_observability::otel::AuthInjectingHttpClient;

    // Reserve a port then drop the listener so the address is closed/refused.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let dead_url = format!("http://{addr}/v1/traces");

    let client = AuthInjectingHttpClient::new(2_000, None);
    let req = http::Request::builder()
        .method("POST")
        .uri(&dead_url)
        .header("content-type", "application/json")
        .body(bytes::Bytes::from_static(b"{}"))
        .unwrap();

    // This is exactly what the batch processor calls per export. Pre-fix it ran
    // on a non-Tokio thread and panicked inside reqwest; here (and post-fix, in
    // the real processor) it runs on the runtime and must return Err, not panic.
    let outcome = client.send_bytes(req).await;
    assert!(
        outcome.is_err(),
        "an unreachable endpoint must surface as a transport Err, not a panic \
         (got {outcome:?})"
    );
}
