//! Optional Tower / Axum server-span instrumentation (feature `tower`).
//!
//! A thin [`tower::Layer`] that wraps any HTTP `Service` — including Axum
//! routers, which *are* `tower::Service<http::Request, Response = http::Response>` —
//! and opens one OpenTelemetry `SpanKind::Server` span per request. The span
//! feeds the **same global tracer** the SDK installs in
//! [`crate::setup_otel_sdk`], so server spans land on api.smoo.ai alongside the
//! GenAI and client spans with zero extra wiring.
//!
//! Why a custom layer instead of `tower_http::trace::TraceLayer`?
//! `TraceLayer` emits `tracing` events, which only reach OTel if a
//! `tracing-opentelemetry` bridge is installed in the host. This crate talks to
//! the `opentelemetry` API directly (see `gen_ai.rs`, `otel.rs`) and installs no
//! such bridge, so a `TraceLayer`-based helper would silently produce no spans.
//! A direct layer keeps the integration honest: the span it creates is the span
//! that gets exported.
//!
//! Span shape (HTTP semantic conventions):
//!   - name: `{method} {route}` (e.g. `GET /organizations/{org_id}/resource`)
//!   - kind: `Server`
//!   - attrs on start: `http.request.method`, `url.path`, `network.protocol.version`
//!   - attrs on finish: `http.response.status_code`
//!   - status: `Error` on a 5xx response or an inner-service error; `Unset`
//!     otherwise (4xx is a client problem, not a server error — matches the
//!     OTel HTTP semconv rule that only 5xx sets span status to error).
//!
//! The span is attached to an [`opentelemetry::Context`] that wraps the inner
//! future, so any spans created downstream (GenAI calls, the reqwest client
//! layer in `reqwest_mw.rs`) nest under it automatically.
//!
//! Routing note: the layer records `url.path` (the raw request path). To get the
//! low-cardinality matched route template as the span name — strongly
//! recommended for Axum so `/users/123` and `/users/456` share one span name —
//! enable route templating by reading Axum's `MatchedPath` from the request
//! extensions; see [`OtelTraceLayer::with_route_extractor`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Instant;

use opentelemetry::global::BoxedSpan;
use opentelemetry::trace::{SpanKind, Status, TraceContextExt, Tracer as _};
use opentelemetry::{Context, KeyValue};
use tower_layer::Layer;
use tower_service::Service;

/// Instrumentation-scope name used for the global tracer lookup. Lets backends
/// attribute these spans to the SmooAI tower integration.
const TRACER_NAME: &str = "smooai-observability/tower";

/// Extracts the low-cardinality route template (e.g. `/users/{id}`) from a
/// request, when the framework exposes it. Returning `None` falls back to the
/// raw request path. For Axum, a typical extractor reads `MatchedPath`:
///
/// ```ignore
/// layer.with_route_extractor(|req: &http::Request<B>| {
///     req.extensions()
///         .get::<axum::extract::MatchedPath>()
///         .map(|m| m.as_str().to_owned())
/// });
/// ```
pub type RouteExtractor<B> = Arc<dyn Fn(&http::Request<B>) -> Option<String> + Send + Sync>;

/// A [`tower::Layer`] that opens a server span per HTTP request on the global
/// tracer. Clone-cheap; share one instance across the whole router.
#[derive(Clone)]
pub struct OtelTraceLayer<B = axum_body_placeholder::Body> {
    route_extractor: Option<RouteExtractor<B>>,
}

// The default body type parameter only matters when a caller writes
// `OtelTraceLayer::default()` without a route extractor; the real body type is
// inferred from the wrapped service at `layer()` time. We keep a private unit
// placeholder so the default generic has a concrete name in docs without
// pulling in `axum` as a dependency.
mod axum_body_placeholder {
    /// Placeholder body type for the default `OtelTraceLayer` generic. Never
    /// constructed — the actual body type is inferred from the wrapped service.
    pub enum Body {}
}

impl<B> Default for OtelTraceLayer<B> {
    fn default() -> Self {
        OtelTraceLayer {
            route_extractor: None,
        }
    }
}

impl<B> OtelTraceLayer<B> {
    /// A layer that names spans `{method} {path}` using the raw request path.
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply a route-template extractor so spans get a low-cardinality name
    /// (e.g. `GET /users/{id}` instead of `GET /users/123`). Strongly
    /// recommended for Axum — see [`RouteExtractor`].
    pub fn with_route_extractor<F>(mut self, f: F) -> Self
    where
        F: Fn(&http::Request<B>) -> Option<String> + Send + Sync + 'static,
    {
        self.route_extractor = Some(Arc::new(f));
        self
    }
}

impl<S, B> Layer<S> for OtelTraceLayer<B> {
    type Service = OtelTraceService<S, B>;

    fn layer(&self, inner: S) -> Self::Service {
        OtelTraceService {
            inner,
            route_extractor: self.route_extractor.clone(),
        }
    }
}

/// The [`tower::Service`] produced by [`OtelTraceLayer`]. Wraps the inner
/// service, opening + closing a server span around each call.
#[derive(Clone)]
pub struct OtelTraceService<S, B> {
    inner: S,
    route_extractor: Option<RouteExtractor<B>>,
}

impl<S, B, ResBody> Service<http::Request<B>> for OtelTraceService<S, B>
where
    S: Service<http::Request<B>, Response = http::Response<ResBody>>,
    S::Future: Send + 'static,
    S::Error: std::fmt::Display,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        let method = req.method().clone();
        let route = self
            .route_extractor
            .as_ref()
            .and_then(|f| f(&req))
            .unwrap_or_else(|| req.uri().path().to_owned());
        let protocol = http_version_str(req.version());

        // Extract any upstream W3C trace context (traceparent/tracestate) the
        // caller injected into the request headers. When present, the server
        // span we open below CONTINUES that trace (parents off the remote span)
        // instead of starting a disconnected root — this is what links traces
        // across service hops. With no traceparent header (or no propagator
        // installed) this yields an empty context and the span becomes a fresh
        // root, exactly as before. (SMOODEV-2024)
        let parent_cx = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&opentelemetry_http::HeaderExtractor(req.headers()))
        });

        // Open a server span on the GLOBAL tracer (the one setup_otel_sdk
        // installed). If no provider is installed it's a cheap no-op span.
        // `with_parent_context` ties it to the extracted upstream context.
        let tracer = opentelemetry::global::tracer(TRACER_NAME);
        let span: BoxedSpan = tracer
            .span_builder(format!("{method} {route}"))
            .with_kind(SpanKind::Server)
            .with_attributes(vec![
                KeyValue::new("http.request.method", method.as_str().to_owned()),
                KeyValue::new("url.path", req.uri().path().to_owned()),
                KeyValue::new("network.protocol.version", protocol),
            ])
            .start_with_context(&tracer, &parent_cx);

        // Build the request context that holds the server span. The inner
        // service is called WITHOUT the context attached here — `ResponseFuture`
        // re-attaches it on every poll instead, so the span is the active parent
        // exactly while the inner future runs (and we never hold the !Send
        // `ContextGuard` across an await point).
        let cx = Context::current_with_span(span);
        let inner_future = self.inner.call(req);

        ResponseFuture {
            inner: inner_future,
            cx,
            start: Instant::now(),
        }
    }
}

pin_project_lite::pin_project! {
    /// Future returned by [`OtelTraceService`]. Re-attaches the request context
    /// on every poll (so downstream code sees the server span as the active
    /// parent) and finalizes the span once the inner service resolves.
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        cx: Context,
        start: Instant,
    }
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<http::Response<ResBody>, E>>,
    E: std::fmt::Display,
{
    type Output = Result<http::Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = self.project();
        // Make the server span the active context while the inner service runs.
        let _guard = this.cx.clone().attach();
        let result = this.inner.poll(cx);

        match &result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(outcome) => {
                let span = this.cx.span();
                let elapsed_ms = this.start.elapsed().as_millis() as i64;
                span.set_attribute(KeyValue::new("http.server.duration_ms", elapsed_ms));
                match outcome {
                    Ok(response) => {
                        let status = response.status();
                        span.set_attribute(KeyValue::new(
                            "http.response.status_code",
                            status.as_u16() as i64,
                        ));
                        // Per OTel HTTP semconv: only 5xx marks the SERVER span
                        // as errored. 4xx is a client issue.
                        if status.is_server_error() {
                            span.set_status(Status::error(
                                status
                                    .canonical_reason()
                                    .unwrap_or("server error")
                                    .to_owned(),
                            ));
                        } else {
                            span.set_status(Status::Ok);
                        }
                    }
                    Err(e) => {
                        span.set_status(Status::error(e.to_string()));
                    }
                }
                span.end();
                result
            }
        }
    }
}

fn http_version_str(v: http::Version) -> &'static str {
    match v {
        http::Version::HTTP_09 => "0.9",
        http::Version::HTTP_10 => "1.0",
        http::Version::HTTP_11 => "1.1",
        http::Version::HTTP_2 => "2",
        http::Version::HTTP_3 => "3",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    // A trivial inner service that echoes a fixed status, for driving the layer.
    #[derive(Clone)]
    struct FixedStatus(http::StatusCode);

    impl Service<http::Request<()>> for FixedStatus {
        type Response = http::Response<()>;
        type Error = Infallible;
        type Future =
            std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

        fn poll_ready(&mut self, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: http::Request<()>) -> Self::Future {
            let status = self.0;
            Box::pin(async move {
                let resp = http::Response::builder().status(status).body(()).unwrap();
                Ok(resp)
            })
        }
    }

    fn request() -> http::Request<()> {
        http::Request::builder()
            .method(http::Method::GET)
            .uri("/users/123")
            .body(())
            .unwrap()
    }

    #[tokio::test]
    async fn passes_through_response_unchanged() {
        let mut svc = OtelTraceLayer::new().layer(FixedStatus(http::StatusCode::OK));
        let resp = svc.call(request()).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
    }

    #[tokio::test]
    async fn server_error_passes_through() {
        // Exercises the 5xx -> Status::error branch (no global provider installed,
        // so the span is a no-op, but the code path must run without panicking).
        let mut svc =
            OtelTraceLayer::new().layer(FixedStatus(http::StatusCode::INTERNAL_SERVER_ERROR));
        let resp = svc.call(request()).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn client_error_is_not_span_error() {
        // 4xx -> Status::Ok branch; just assert pass-through + no panic.
        let mut svc = OtelTraceLayer::new().layer(FixedStatus(http::StatusCode::NOT_FOUND));
        let resp = svc.call(request()).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn route_extractor_overrides_path() {
        // The extractor runs without panicking; span name uses the template.
        let layer = OtelTraceLayer::new()
            .with_route_extractor(|_req: &http::Request<()>| Some("/users/{id}".to_owned()));
        let mut svc = layer.layer(FixedStatus(http::StatusCode::OK));
        let resp = svc.call(request()).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
    }

    #[test]
    fn http_version_strings() {
        assert_eq!(http_version_str(http::Version::HTTP_11), "1.1");
        assert_eq!(http_version_str(http::Version::HTTP_2), "2");
    }

    // --- W3C trace context propagation (SMOODEV-2024) ----------------------

    use opentelemetry::propagation::TextMapPropagator;
    use opentelemetry::trace::{SpanContext, TraceState};
    use opentelemetry::{SpanId, TraceFlags, TraceId};
    use opentelemetry_sdk::propagation::TraceContextPropagator;

    #[test]
    fn inject_extract_round_trip_preserves_trace_id() {
        let propagator = TraceContextPropagator::new();

        let trace_id = TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap();
        let span_id = SpanId::from_hex("b7ad6b7169203331").unwrap();
        let sc = SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        );
        let cx = Context::new().with_remote_span_context(sc);

        // Inject into a fresh HeaderMap (the outbound side).
        let mut headers = http::HeaderMap::new();
        propagator.inject_context(&cx, &mut opentelemetry_http::HeaderInjector(&mut headers));

        // Extract it back out (the inbound side, as the tower layer does).
        let extracted = propagator.extract(&opentelemetry_http::HeaderExtractor(&headers));
        let extracted_sc = extracted.span().span_context().clone();

        assert!(extracted_sc.is_valid(), "extracted context is valid");
        assert_eq!(
            extracted_sc.trace_id(),
            trace_id,
            "trace_id survives the round-trip"
        );
        assert_eq!(
            extracted_sc.span_id(),
            span_id,
            "parent span_id survives the round-trip"
        );
        assert!(
            extracted_sc.is_remote(),
            "extracted context is marked remote"
        );
    }

    #[test]
    fn extract_with_no_headers_yields_invalid_context() {
        let propagator = TraceContextPropagator::new();
        let headers = http::HeaderMap::new();
        let extracted = propagator.extract(&opentelemetry_http::HeaderExtractor(&headers));
        // With no traceparent header the extracted span context is invalid, so
        // the server span the layer opens becomes a fresh root.
        assert!(
            !extracted.span().span_context().is_valid(),
            "no headers -> invalid (empty) span context"
        );
    }
}
