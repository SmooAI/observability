//! Optional reqwest client-span instrumentation (feature `reqwest-middleware`).
//!
//! A [`reqwest_middleware::Middleware`] that opens one OpenTelemetry
//! `SpanKind::Client` span per outbound HTTP call, feeding the **same global
//! tracer** the SDK installs in [`crate::setup_otel_sdk`]. Drop it into a
//! `reqwest_middleware::ClientBuilder` and every request the client makes
//! becomes a client span — nesting under the current server span when one is
//! active (e.g. inside a handler wrapped by the `tower` layer).
//!
//! Span shape (HTTP semantic conventions):
//!   - name: `{method}` (e.g. `GET`) — keeps cardinality low; the URL goes in
//!     attributes, not the name, per OTel HTTP client semconv.
//!   - kind: `Client`
//!   - attrs on start: `http.request.method`, `url.full`, `server.address`,
//!     `server.port`
//!   - attrs on finish: `http.response.status_code`
//!   - status: `Error` on a transport error or a 4xx/5xx response; `Ok`
//!     otherwise. (Unlike the server span, a CLIENT span treats any 4xx/5xx as
//!     an error for the caller — the OTel client semconv marks the span errored
//!     for status >= 400.)
//!
//! ## Why a dedicated middleware (not a thin span-wrapping helper)
//!
//! `reqwest-middleware` 0.5 supports reqwest 0.13 (this crate's pin), so the
//! middleware path *is* cleanly feasible — it intercepts at the client's own
//! extension point, so retries / redirects the real reqwest client performs are
//! all covered by the one span. For callers who don't want the
//! `reqwest-middleware` dependency, [`instrument_client_call`] offers a manual
//! span-wrapping alternative around any future.

use opentelemetry::trace::{FutureExt as _, SpanKind, Status, TraceContextExt, Tracer as _};
use opentelemetry::{Context, KeyValue};
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};

/// Instrumentation-scope name used for the global tracer lookup.
const TRACER_NAME: &str = "smooai-observability/reqwest";

/// reqwest middleware that wraps each outbound call in a client span on the
/// global tracer. Zero-config — `OtelReqwestMiddleware::default()` is all most
/// callers need.
///
/// ```ignore
/// use reqwest_middleware::ClientBuilder;
/// use smooai_observability::reqwest_mw::OtelReqwestMiddleware;
///
/// let client = ClientBuilder::new(reqwest::Client::new())
///     .with(OtelReqwestMiddleware::default())
///     .build();
/// ```
#[derive(Clone, Default)]
pub struct OtelReqwestMiddleware {
    _private: (),
}

impl OtelReqwestMiddleware {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl Middleware for OtelReqwestMiddleware {
    async fn handle(
        &self,
        mut req: Request,
        extensions: &mut http::Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        let method = req.method().clone();
        let url = req.url().clone();

        let tracer = opentelemetry::global::tracer(TRACER_NAME);
        let mut attributes = vec![
            KeyValue::new("http.request.method", method.as_str().to_owned()),
            KeyValue::new("url.full", url.as_str().to_owned()),
        ];
        if let Some(host) = url.host_str() {
            attributes.push(KeyValue::new("server.address", host.to_owned()));
        }
        if let Some(port) = url.port_or_known_default() {
            attributes.push(KeyValue::new("server.port", port as i64));
        }

        let span = tracer
            .span_builder(method.as_str().to_owned())
            .with_kind(SpanKind::Client)
            .with_attributes(attributes)
            .start(&tracer);

        // Run the rest of the chain with the client span as the active context,
        // so the real reqwest request (and any nested instrumentation) sees it.
        // `with_context` (vs `attach`) keeps the future `Send` — `ContextGuard`
        // is `!Send` and can't be held across this `.await`.
        let cx = Context::current_with_span(span);

        // Inject W3C trace context (traceparent/tracestate) into the outbound
        // request headers so the downstream service can EXTRACT it and continue
        // this trace instead of starting a disconnected root. We inject the
        // freshly-created client span's context, so downstream server spans
        // parent off this CLIENT span — yielding a single linked trace across
        // the service hop. If no propagator is installed (it always is once
        // `setup_otel_sdk` ran) this is a no-op. (SMOODEV-2024)
        opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.inject_context(
                &cx,
                &mut opentelemetry_http::HeaderInjector(req.headers_mut()),
            );
        });

        let result = next.run(req, extensions).with_context(cx.clone()).await;

        let span = cx.span();
        match &result {
            Ok(response) => {
                let status = response.status();
                span.set_attribute(KeyValue::new(
                    "http.response.status_code",
                    status.as_u16() as i64,
                ));
                // Client semconv: any >= 400 status is an error for the caller.
                if status.is_client_error() || status.is_server_error() {
                    span.set_status(Status::error(
                        status.canonical_reason().unwrap_or("http error").to_owned(),
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

/// Manual span-wrapping alternative for callers who do NOT want the
/// `reqwest-middleware` machinery (or aren't using a `ClientWithMiddleware`).
/// Wraps any future representing one outbound call in a client span on the
/// global tracer, recording method + URL up front and the result on completion.
///
/// ```ignore
/// let resp = instrument_client_call(
///     reqwest::Method::GET,
///     "https://api.smoo.ai/v1/models",
///     client.get("https://api.smoo.ai/v1/models").send(),
/// )
/// .await?;
/// ```
pub async fn instrument_client_call<F, T, E>(
    method: reqwest::Method,
    url: impl Into<String>,
    fut: F,
) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
    T: HttpStatus,
{
    let url = url.into();
    let tracer = opentelemetry::global::tracer(TRACER_NAME);
    let span = tracer
        .span_builder(method.as_str().to_owned())
        .with_kind(SpanKind::Client)
        .with_attributes(vec![
            KeyValue::new("http.request.method", method.as_str().to_owned()),
            KeyValue::new("url.full", url),
        ])
        .start(&tracer);

    let cx = Context::current_with_span(span);
    let result = fut.with_context(cx.clone()).await;

    let span = cx.span();
    match &result {
        Ok(value) => {
            if let Some(code) = value.status_code() {
                span.set_attribute(KeyValue::new("http.response.status_code", code as i64));
                if code >= 400 {
                    span.set_status(Status::error("http error"));
                } else {
                    span.set_status(Status::Ok);
                }
            } else {
                span.set_status(Status::Ok);
            }
        }
        Err(e) => span.set_status(Status::error(e.to_string())),
    }
    span.end();
    result
}

/// Lets [`instrument_client_call`] read a status code off the success value
/// (e.g. a [`reqwest::Response`]) without knowing the concrete type. Implement
/// it for your own response type, or rely on the blanket impls below.
pub trait HttpStatus {
    /// The HTTP status code, if this value carries one.
    fn status_code(&self) -> Option<u16>;
}

impl HttpStatus for Response {
    fn status_code(&self) -> Option<u16> {
        Some(self.status().as_u16())
    }
}

impl HttpStatus for () {
    fn status_code(&self) -> Option<u16> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn instrument_client_call_ok_path() {
        // No global provider installed -> no-op span, but the code path (Ok,
        // status_code None) must run without panicking and return the value.
        let out: Result<(), std::io::Error> =
            instrument_client_call(reqwest::Method::GET, "https://example.com", async {
                Ok(())
            })
            .await;
        assert!(out.is_ok());
    }

    #[tokio::test]
    async fn instrument_client_call_err_path() {
        let out: Result<(), std::io::Error> =
            instrument_client_call(reqwest::Method::POST, "https://example.com", async {
                Err(std::io::Error::other("connection reset"))
            })
            .await;
        assert!(out.is_err());
    }

    #[test]
    fn unit_has_no_status() {
        assert_eq!(HttpStatus::status_code(&()), None);
    }

    #[test]
    fn middleware_is_constructible() {
        let _m = OtelReqwestMiddleware::new();
    }

    // --- W3C trace context propagation (SMOODEV-2024) ----------------------
    //
    // These exercise the inject side of the middleware's behavior directly via
    // the global propagator, independent of a live reqwest call. The
    // inbound/extract counterpart lives in `tower.rs`; a full inject->extract
    // round-trip is covered there too.

    use opentelemetry::propagation::TextMapPropagator;
    use opentelemetry::trace::{SpanContext, TraceContextExt, TraceState};
    use opentelemetry::{Context, SpanId, TraceFlags, TraceId};
    use opentelemetry_sdk::propagation::TraceContextPropagator;

    /// A context carrying a known sampled remote span context.
    fn known_context() -> (Context, TraceId, SpanId) {
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
        (cx, trace_id, span_id)
    }

    #[test]
    fn inject_writes_traceparent_header() {
        let propagator = TraceContextPropagator::new();
        let (cx, trace_id, span_id) = known_context();

        let mut headers = http::HeaderMap::new();
        propagator.inject_context(&cx, &mut opentelemetry_http::HeaderInjector(&mut headers));

        let traceparent = headers
            .get("traceparent")
            .expect("traceparent header injected")
            .to_str()
            .unwrap();
        // W3C format: 00-<trace_id>-<span_id>-<flags>
        assert_eq!(
            traceparent,
            format!("00-{trace_id}-{span_id}-01"),
            "traceparent encodes the active span context"
        );
    }

    #[test]
    fn inject_with_no_active_context_is_noop() {
        let propagator = TraceContextPropagator::new();
        // An empty context has no valid span context, so nothing is injected.
        let cx = Context::new();
        let mut headers = http::HeaderMap::new();
        propagator.inject_context(&cx, &mut opentelemetry_http::HeaderInjector(&mut headers));
        assert!(
            headers.get("traceparent").is_none(),
            "no traceparent header when there is no active span context"
        );
    }
}
