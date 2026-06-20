//! Custom [`opentelemetry_http::HttpClient`] that injects a fresh Bearer token
//! on every export, mirroring the TS `auth-injecting-exporter.ts`.
//!
//! The upstream OTLP exporter holds its HTTP client + headers for the life of
//! the process, so a token minted once at setup goes stale after ~1h and every
//! subsequent export 401s (the exact bug SMOODEV-1206 fixed on the TS side).
//! This wrapper closes the same gap for Rust: each `send_bytes`
//!   1. asks the [`TokenProvider`] for a token (cache-hit if not near expiry),
//!   2. sets `authorization: Bearer <token>` on the outgoing request,
//!   3. on a 401 response, invalidates the cached token + retries ONCE.
//!
//! When no `TokenProvider` is configured the wrapper is a transparent
//! pass-through to the inner `reqwest::Client` (static-header / no-auth mode).

use crate::auth::TokenProvider;
use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response};
use opentelemetry_http::{HttpClient, HttpError};

/// HTTP client wrapper that injects M2M auth per request. Cloneable; clones
/// share the inner client + token cache.
#[derive(Debug, Clone)]
pub struct AuthInjectingHttpClient {
    inner: reqwest::Client,
    token_provider: Option<TokenProvider>,
}

impl AuthInjectingHttpClient {
    pub fn new(inner: reqwest::Client, token_provider: Option<TokenProvider>) -> Self {
        AuthInjectingHttpClient {
            inner,
            token_provider,
        }
    }

    /// Clone a `Request<Bytes>` (parts + cheap `Bytes` body clone) so the
    /// 401-retry path can resend after re-minting the token.
    fn clone_request(req: &Request<Bytes>) -> Request<Bytes> {
        let mut builder = Request::builder()
            .method(req.method().clone())
            .uri(req.uri().clone());
        if let Some(headers) = builder.headers_mut() {
            *headers = req.headers().clone();
        }
        // Unwrap is safe: we rebuilt from a valid request's parts.
        builder
            .body(req.body().clone())
            .expect("rebuilding a valid request must not fail")
    }

    fn set_bearer(req: &mut Request<Bytes>, token: &str) {
        if let Ok(value) = http::HeaderValue::from_str(&format!("Bearer {token}")) {
            req.headers_mut().insert(http::header::AUTHORIZATION, value);
        }
    }
}

#[async_trait]
impl HttpClient for AuthInjectingHttpClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        // No token provider → transparent pass-through (static-header mode).
        let Some(provider) = &self.token_provider else {
            return self.inner.send_bytes(request).await;
        };

        // Keep a copy for the potential 401 retry before we consume `request`.
        let retry_template = Self::clone_request(&request);

        let mut req = request;
        match provider.get_access_token().await {
            Ok(token) => Self::set_bearer(&mut req, &token),
            Err(e) => {
                // Couldn't mint — send unauthenticated; the server 401 is more
                // useful in logs than silently dropping. Never panic.
                crate::otel::warn(&format!("token mint failed before export: {e}"));
            }
        }

        let result = self.inner.send_bytes(req).await;

        // 401 retry: token may have been revoked / rotated server-side. Drop the
        // cached value and re-mint once. Don't loop.
        //
        // NOTE: the upstream `HttpClient for reqwest::Client` impl calls
        // `.error_for_status()`, so a 401 surfaces as `Err(reqwest::Error)` —
        // NOT `Ok(response_with_status_401)`. We therefore detect the 401 in
        // BOTH shapes: the error path (downcast to reqwest::Error + check
        // `.status()`) and, defensively, an `Ok` whose status is 401 (in case a
        // future inner client doesn't auto-error).
        let is_401 = match &result {
            Ok(resp) => resp.status() == http::StatusCode::UNAUTHORIZED,
            Err(e) => e
                .downcast_ref::<reqwest::Error>()
                .and_then(|re| re.status())
                .map(|s| s == reqwest::StatusCode::UNAUTHORIZED)
                .unwrap_or(false),
        };

        if is_401 {
            provider.invalidate().await;
            let mut retry = retry_template;
            if let Ok(token) = provider.get_access_token().await {
                Self::set_bearer(&mut retry, &token);
            }
            return self.inner.send_bytes(retry).await;
        }

        let response = result?;

        Ok(response)
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
        let client = AuthInjectingHttpClient::new(reqwest::Client::new(), None);
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
        let client = AuthInjectingHttpClient::new(reqwest::Client::new(), Some(tp));
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
        let client = AuthInjectingHttpClient::new(reqwest::Client::new(), Some(tp));
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
}
