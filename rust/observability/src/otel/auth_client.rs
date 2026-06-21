//! [`opentelemetry_http::HttpClient`] backed by `smooai-fetch`, injecting a
//! fresh Bearer token on every export — mirroring the TS
//! `auth-injecting-exporter.ts` and routing the OTLP protocol transport through
//! the same resilient client (`timeouts + retries + circuit breaking`) every
//! other SmooAI outbound call uses (SMOODEV-2029).
//!
//! ## Why this exists
//!
//! The upstream OTLP exporter holds its HTTP client + headers for the life of
//! the process, so a token minted once at setup goes stale after ~1h and every
//! subsequent export 401s (the exact bug SMOODEV-1206 fixed on the TS side).
//! This wrapper closes the same gap for Rust: each `send_bytes`
//!   1. asks the [`TokenProvider`] for a token (cache-hit if not near expiry),
//!   2. sets `authorization: Bearer <token>` on the outgoing request,
//!   3. on a 401 response, invalidates the cached token + retries ONCE.
//!
//! When no `TokenProvider` is configured the wrapper sends the request as-is
//! (static-header / no-auth mode — the static `authorization` header, if any,
//! is merged onto the request by the OTLP exporter before it reaches us).
//!
//! ## Retry ownership — avoiding double-retry (SMOODEV-2029)
//!
//! `smooai-fetch` OWNS transport retry: it retries 429 / 5xx / timeouts /
//! connection errors with exponential backoff + `Retry-After` honoring + circuit
//! breaking. The OTLP exporter layer must NOT also retry the same failure. That
//! is guaranteed here because this crate does NOT enable opentelemetry-otlp's
//! `experimental-http-retry` feature — with that feature off, the exporter calls
//! `send_bytes` exactly once per export and never retries (see
//! `opentelemetry-otlp`'s `export_http_once` no-retry path). So smooai-fetch is
//! the single retry layer for 429/5xx/transport, and the only retry WE add is the
//! 401→invalidate→re-mint→retry-once below — which smooai-fetch deliberately does
//! NOT do (401 is not in its retryable set), so the two never stack.
//!
//! ## Response translation
//!
//! `opentelemetry_http::HttpClient::send_bytes` returns `Response<Bytes>` for any
//! completed HTTP exchange — including non-2xx — because the OTLP exporter reads
//! `response.status()` itself to decide success/failure. `smooai-fetch`, by
//! contrast, returns `Err(FetchError::HttpResponse { .. })` for non-2xx. We
//! therefore reconstruct an `http::Response` (status + headers + body) from that
//! error variant so the exporter sees the real status, and surface only genuine
//! transport failures (timeout, exhausted retries, connection error) as `Err`.

use crate::auth::TokenProvider;
use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response};
use opentelemetry_http::{HttpClient, HttpError};
use smooai_fetch::defaults::default_retry_options;
use smooai_fetch::error::FetchError;
use smooai_fetch::response::FetchResponse;
use smooai_fetch::types::{Method, RequestInit};
use smooai_fetch::{FetchBuilder, FetchClient};
use std::collections::HashMap;
use std::sync::Arc;

/// HTTP client wrapper that sends OTLP exports through `smooai-fetch` and injects
/// M2M auth per request. Cheap to clone (`Arc`-shared); clones share the inner
/// fetch client + token cache.
#[derive(Clone)]
pub struct AuthInjectingHttpClient {
    // Typed to `serde_json::Value` because OTLP/HTTP/JSON responses are JSON; the
    // parsed body is unused (we reconstruct the response from the raw `body`
    // string), but the type makes a 2xx JSON body parse cleanly rather than trip
    // smooai-fetch's schema-validation guard.
    http: Arc<FetchClient<serde_json::Value>>,
    token_provider: Option<TokenProvider>,
}

// `opentelemetry_http::HttpClient` requires `Debug`. `FetchClient<T>` doesn't
// implement it, and the `TokenProvider` Debug is hand-rolled to redact the
// client secret — so derive nothing and print only the auth-mode flag.
impl std::fmt::Debug for AuthInjectingHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthInjectingHttpClient")
            .field("auth", &self.token_provider.is_some())
            .finish_non_exhaustive()
    }
}

impl AuthInjectingHttpClient {
    /// Build the adapter. `timeout_ms` and the retry policy are baked into the
    /// underlying `smooai-fetch` client (this is the SINGLE retry layer — the
    /// OTLP exporter does not retry; see the module docs).
    pub fn new(timeout_ms: u64, token_provider: Option<TokenProvider>) -> Self {
        let http = FetchBuilder::<serde_json::Value>::new()
            .with_timeout(timeout_ms)
            .with_retry(default_retry_options())
            .build();
        AuthInjectingHttpClient {
            http: Arc::new(http),
            token_provider,
        }
    }

    /// Translate an `http::Request<Bytes>` (what `opentelemetry-http` hands us)
    /// into a `smooai-fetch` `(url, RequestInit)`. The OTLP exporter only ever
    /// POSTs, but we map the method faithfully. The body for OTLP/HTTP/JSON is
    /// UTF-8 JSON, so `Bytes` → `String` is lossless; a non-UTF-8 body would be a
    /// protocol bug, and `from_utf8_lossy` keeps us from panicking on it.
    fn to_fetch(req: &Request<Bytes>, bearer: Option<&str>) -> (String, RequestInit) {
        let url = req.uri().to_string();
        let method = match *req.method() {
            http::Method::POST => Method::POST,
            http::Method::PUT => Method::PUT,
            http::Method::PATCH => Method::PATCH,
            http::Method::DELETE => Method::DELETE,
            http::Method::HEAD => Method::HEAD,
            http::Method::OPTIONS => Method::OPTIONS,
            _ => Method::GET,
        };
        let mut headers = HashMap::new();
        for (name, value) in req.headers().iter() {
            if let Ok(v) = value.to_str() {
                headers.insert(name.as_str().to_string(), v.to_string());
            }
        }
        // Inject / override the Authorization header with the fresh token.
        if let Some(token) = bearer {
            headers.insert("authorization".to_string(), format!("Bearer {token}"));
        }
        let body = String::from_utf8_lossy(req.body()).into_owned();
        (
            url,
            RequestInit {
                method,
                headers,
                body: Some(body),
            },
        )
    }

    /// Rebuild an `http::Response<Bytes>` from a (status, headers, body) triple —
    /// used for BOTH the success path (`FetchResponse`) and the non-2xx path
    /// (`FetchError::HttpResponse`), so the OTLP exporter always sees the real
    /// status it inspects.
    fn build_response(
        status: u16,
        headers: &HashMap<String, String>,
        body: String,
    ) -> Result<Response<Bytes>, HttpError> {
        let mut builder = Response::builder().status(status);
        for (k, v) in headers.iter() {
            if let (Ok(name), Ok(value)) = (
                http::header::HeaderName::from_bytes(k.as_bytes()),
                http::header::HeaderValue::from_str(v),
            ) {
                builder = builder.header(name, value);
            }
        }
        builder
            .body(Bytes::from(body.into_bytes()))
            .map_err(Into::into)
    }

    /// Map a `smooai-fetch` result into the `Response<Bytes>` the OTLP exporter
    /// expects. A non-2xx surfaces from `smooai-fetch` as
    /// `Err(FetchError::HttpResponse { .. })` — we turn that back into a real
    /// `Response` (preserving status/headers/body). The retry loop wraps the
    /// final failure in `FetchError::Retry`, so unwrap one level to recover the
    /// underlying response. Everything else (timeout, transport, exhausted
    /// non-HTTP retries) is a genuine transport failure → `Err(HttpError)`.
    fn into_http_response(
        result: Result<FetchResponse<serde_json::Value>, FetchError>,
    ) -> Result<Response<Bytes>, HttpError> {
        match result {
            Ok(resp) => Self::build_response(resp.status, &resp.headers, resp.body),
            Err(err) => {
                let unwrapped = match &err {
                    FetchError::Retry { source, .. } => source.as_ref(),
                    other => other,
                };
                match unwrapped {
                    FetchError::HttpResponse {
                        status,
                        headers,
                        body,
                        ..
                    } => Self::build_response(*status, headers, body.clone()),
                    _ => Err(Box::new(err)),
                }
            }
        }
    }

    /// Did this fetch result resolve to an HTTP 401? smooai-fetch reports non-2xx
    /// as `Err(HttpResponse { status: 401, .. })`; defensively also catch an `Ok`
    /// whose status is 401 (it shouldn't be — 401 isn't 2xx — but a future
    /// smooai-fetch could change that).
    fn is_unauthorized(result: &Result<FetchResponse<serde_json::Value>, FetchError>) -> bool {
        match result {
            Ok(resp) => resp.status == 401,
            Err(err) => {
                let unwrapped = match err {
                    FetchError::Retry { source, .. } => source.as_ref(),
                    other => other,
                };
                matches!(unwrapped, FetchError::HttpResponse { status: 401, .. })
            }
        }
    }
}

#[async_trait]
impl HttpClient for AuthInjectingHttpClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        // No token provider → send as-is (static-header / no-auth mode).
        let Some(provider) = &self.token_provider else {
            let (url, init) = Self::to_fetch(&request, None);
            return Self::into_http_response(self.http.fetch(&url, init).await);
        };

        // Mint a token for this export. A mint failure is logged, not fatal — we
        // still send (unauthenticated) so the server's 401 surfaces in logs
        // rather than silently dropping the batch. Never panic.
        let token = match provider.get_access_token().await {
            Ok(token) => Some(token),
            Err(e) => {
                crate::otel::warn(&format!("token mint failed before export: {e}"));
                None
            }
        };

        let (url, init) = Self::to_fetch(&request, token.as_deref());
        let result = self.http.fetch(&url, init).await;

        // 401 retry: the token may have been revoked / rotated server-side. Drop
        // the cached value, re-mint, and retry ONCE. smooai-fetch does not retry
        // 401 itself (not in its retryable set), so this is the only 401 retry —
        // no double-retry. Don't loop.
        if Self::is_unauthorized(&result) {
            provider.invalidate().await;
            let retry_token = provider.get_access_token().await.ok();
            let (retry_url, retry_init) = Self::to_fetch(&request, retry_token.as_deref());
            return Self::into_http_response(self.http.fetch(&retry_url, retry_init).await);
        }

        Self::into_http_response(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{TokenProvider, TokenProviderOptions};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request as WmRequest, Respond, ResponseTemplate};

    const TEST_TIMEOUT_MS: u64 = 10_000;

    fn build_req(uri: &str) -> Request<Bytes> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Bytes::from_static(b"{}"))
            .unwrap()
    }

    #[tokio::test]
    async fn passthrough_without_provider() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let client = AuthInjectingHttpClient::new(TEST_TIMEOUT_MS, None);
        let resp = client
            .send_bytes(build_req(&format!("{}/v1/traces", server.uri())))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn injects_bearer_token() {
        let auth = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "tok-xyz",
                "expires_in": 3600
            })))
            .mount(&auth)
            .await;

        // Assert the export request carried the Authorization header.
        struct AssertAuth;
        impl Respond for AssertAuth {
            fn respond(&self, req: &WmRequest) -> ResponseTemplate {
                let got = req
                    .headers
                    .get("authorization")
                    .map(|v| v.to_str().unwrap_or("").to_string());
                if got.as_deref() == Some("Bearer tok-xyz") {
                    ResponseTemplate::new(200)
                } else {
                    ResponseTemplate::new(400)
                }
            }
        }
        let ingest = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .respond_with(AssertAuth)
            .mount(&ingest)
            .await;

        let tp = TokenProvider::new(TokenProviderOptions::new(auth.uri(), "cid", "sk")).unwrap();
        let client = AuthInjectingHttpClient::new(TEST_TIMEOUT_MS, Some(tp));
        let resp = client
            .send_bytes(build_req(&format!("{}/v1/traces", ingest.uri())))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            200,
            "Authorization header should have been injected"
        );
    }

    #[tokio::test]
    async fn retries_once_on_401() {
        let auth = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "tok-1",
                "expires_in": 3600
            })))
            .mount(&auth)
            .await;

        // First export call → 401, second → 200.
        let hits = Arc::new(AtomicUsize::new(0));
        struct Flaky(Arc<AtomicUsize>);
        impl Respond for Flaky {
            fn respond(&self, _req: &WmRequest) -> ResponseTemplate {
                let n = self.0.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    ResponseTemplate::new(401)
                } else {
                    ResponseTemplate::new(200)
                }
            }
        }
        let ingest = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .respond_with(Flaky(hits.clone()))
            .mount(&ingest)
            .await;

        let tp = TokenProvider::new(TokenProviderOptions::new(auth.uri(), "cid", "sk")).unwrap();
        let client = AuthInjectingHttpClient::new(TEST_TIMEOUT_MS, Some(tp));
        let resp = client
            .send_bytes(build_req(&format!("{}/v1/traces", ingest.uri())))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "should have retried exactly once"
        );
    }

    #[tokio::test]
    async fn non_2xx_surfaces_as_response_not_error() {
        // A 4xx that is NOT 401 must come back as a `Response` carrying the real
        // status (so the OTLP exporter can read it), not a transport `Err`.
        let ingest = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .respond_with(ResponseTemplate::new(422).set_body_string("bad payload"))
            .mount(&ingest)
            .await;

        let client = AuthInjectingHttpClient::new(TEST_TIMEOUT_MS, None);
        let resp = client
            .send_bytes(build_req(&format!("{}/v1/traces", ingest.uri())))
            .await
            .expect("non-2xx should be a Response, not an Err");
        assert_eq!(resp.status(), 422);
    }
}
