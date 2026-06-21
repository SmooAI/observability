//! OAuth2 `client_credentials` token provider — direct port of the TS
//! `auth/token-provider.ts` so the Rust SDK authenticates against api.smoo.ai
//! exactly the same way every other SmooAI client does.
//!
//! The token is consulted at *request* time by the OTLP exporter's auth-
//! injecting HTTP client — no snapshot, no staleness. Cached in memory until
//! `refresh_window` before expiry, then refreshed. Concurrent callers during a
//! refresh share one in-flight request (a `tokio::sync::Mutex` guards the
//! refresh so duplicate token exchanges don't churn the rate limiter).
//!
//! The token exchange goes through `smooai-fetch` (timeouts + retries + circuit
//! breaking) rather than raw `reqwest` (SMOODEV-2026).
//!
//! Server contract:
//!
//! ```text
//! POST {auth_url}/token
//! Content-Type: application/x-www-form-urlencoded
//!
//! grant_type=client_credentials
//! provider=client_credentials
//! client_id=<uuid>
//! client_secret=sk_...
//! ```

use serde::Deserialize;
use smooai_fetch::defaults::default_retry_options;
use smooai_fetch::error::FetchError;
use smooai_fetch::types::{Method, RequestInit};
use smooai_fetch::{FetchBuilder, FetchClient};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// Errors from the token exchange. Callers in the export path log + drop these;
/// observability must never panic the host.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("token http error: {0}")]
    Http(String),
    #[error("token exchange failed: HTTP {status} {body}")]
    Status { status: u16, body: String },
    #[error("token endpoint returned no access_token")]
    NoAccessToken,
    #[error("missing config: {0}")]
    Config(&'static str),
}

// `Clone` is required by smooai-fetch's `FetchClient<T>` bound
// (`DeserializeOwned + Clone + Send + 'static`).
#[derive(Deserialize, Clone)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Clone)]
struct CachedToken {
    access_token: String,
    /// Unix epoch seconds when the token expires.
    expires_at: u64,
}

/// Options for [`TokenProvider::new`].
#[derive(Clone)]
pub struct TokenProviderOptions {
    /// OAuth issuer base URL, e.g. `https://auth.smoo.ai`. Trailing slashes are
    /// trimmed.
    pub auth_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// Seconds before expiry to proactively refresh. Defaults to 60s — matches
    /// the TS TokenProvider.
    pub refresh_window_secs: u64,
}

impl TokenProviderOptions {
    pub fn new(
        auth_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        TokenProviderOptions {
            auth_url: auth_url.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            refresh_window_secs: 60,
        }
    }
}

struct Inner {
    auth_url: String,
    client_id: String,
    client_secret: String,
    refresh_window_secs: u64,
    // smooai-fetch client typed to the token JSON so a successful exchange parses
    // straight into `FetchResponse::data`. Carries the 10s timeout + retry policy.
    http: FetchClient<TokenResponse>,
    cached: Mutex<Option<CachedToken>>,
}

/// M2M token provider. Cheap to clone (`Arc`-shared); clones share the cache.
#[derive(Clone)]
pub struct TokenProvider {
    inner: Arc<Inner>,
}

// Manual Debug that never prints the client secret or cached token — a derived
// impl would risk leaking credentials into logs / panic messages.
impl std::fmt::Debug for TokenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenProvider")
            .field("auth_url", &self.inner.auth_url)
            .field("client_id", &self.inner.client_id)
            .field("client_secret", &"[redacted]")
            .field("refresh_window_secs", &self.inner.refresh_window_secs)
            .finish_non_exhaustive()
    }
}

impl TokenProvider {
    /// Construct a provider. Returns an error only on empty required config —
    /// matches the TS constructor's throws.
    pub fn new(options: TokenProviderOptions) -> Result<Self, TokenError> {
        if options.auth_url.is_empty() {
            return Err(TokenError::Config("authUrl"));
        }
        if options.client_id.is_empty() {
            return Err(TokenError::Config("clientId"));
        }
        if options.client_secret.is_empty() {
            return Err(TokenError::Config("clientSecret"));
        }
        let http = FetchBuilder::<TokenResponse>::new()
            .with_timeout(10_000)
            .with_retry(default_retry_options())
            .build();
        Ok(TokenProvider {
            inner: Arc::new(Inner {
                auth_url: options.auth_url.trim_end_matches('/').to_string(),
                client_id: options.client_id,
                client_secret: options.client_secret,
                refresh_window_secs: options.refresh_window_secs,
                http,
                cached: Mutex::new(None),
            }),
        })
    }

    /// Returns a valid OAuth access token, refreshing if the cached value is
    /// missing, expired, or within `refresh_window_secs` of expiry. The cache
    /// lock is held across the refresh so concurrent callers don't fire
    /// duplicate token exchanges.
    pub async fn get_access_token(&self) -> Result<String, TokenError> {
        let mut guard = self.inner.cached.lock().await;
        if let Some(cached) = guard.as_ref() {
            if !self.should_refresh(cached) {
                return Ok(cached.access_token.clone());
            }
        }
        let fresh = self.refresh().await?;
        let token = fresh.access_token.clone();
        *guard = Some(fresh);
        Ok(token)
    }

    /// Drop the cached token. Call when an export observes a 401 so the next
    /// attempt re-mints.
    pub async fn invalidate(&self) {
        let mut guard = self.inner.cached.lock().await;
        *guard = None;
    }

    fn should_refresh(&self, cached: &CachedToken) -> bool {
        let now = now_secs();
        now >= cached
            .expires_at
            .saturating_sub(self.inner.refresh_window_secs)
    }

    async fn refresh(&self) -> Result<CachedToken, TokenError> {
        // smooai-fetch's `RequestInit` takes a pre-serialized String body, so
        // build the `application/x-www-form-urlencoded` payload by hand. Each
        // value is percent-encoded (the `client_secret` can contain reserved
        // characters); the keys are all token-safe literals.
        let body = format!(
            "grant_type=client_credentials&provider=client_credentials&client_id={}&client_secret={}",
            form_encode(&self.inner.client_id),
            form_encode(&self.inner.client_secret),
        );

        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        );

        let resp = self
            .inner
            .http
            .fetch(
                &format!("{}/token", self.inner.auth_url),
                RequestInit {
                    method: Method::POST,
                    headers,
                    body: Some(body),
                },
            )
            .await
            .map_err(token_error_from_fetch)?;

        // 2xx only reaches here (non-2xx is an `Err` from `fetch`). The typed
        // client parsed the JSON into `resp.data`.
        let parsed = resp.data.ok_or(TokenError::NoAccessToken)?;
        let access_token = parsed.access_token.ok_or(TokenError::NoAccessToken)?;
        let expires_in = parsed.expires_in.unwrap_or(3600);
        Ok(CachedToken {
            access_token,
            expires_at: now_secs() + expires_in,
        })
    }
}

/// Map a [`FetchError`] onto a [`TokenError`]. A non-2xx response surfaces as
/// `TokenError::Status` (preserving the upstream status + body); everything else
/// (timeout, transport, exhausted retries) folds into `TokenError::Http`. The
/// retry loop wraps the final failure in `FetchError::Retry`, so unwrap one level
/// to recover an underlying `HttpResponse` status.
fn token_error_from_fetch(err: FetchError) -> TokenError {
    let unwrapped = match &err {
        FetchError::Retry { source, .. } => source.as_ref(),
        other => other,
    };
    match unwrapped {
        FetchError::HttpResponse { status, body, .. } => TokenError::Status {
            status: *status,
            body: body.clone(),
        },
        _ => TokenError::Http(err.to_string()),
    }
}

/// Minimal `application/x-www-form-urlencoded` value encoder: percent-encodes
/// everything outside the RFC 3986 unreserved set (`A-Z a-z 0-9 - . _ ~`).
fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn mints_and_caches_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "tok-123",
                "expires_in": 3600
            })))
            .expect(1) // cached on second call → only one HTTP hit
            .mount(&server)
            .await;

        let tp = TokenProvider::new(TokenProviderOptions::new(server.uri(), "cid", "sk_secret"))
            .unwrap();
        assert_eq!(tp.get_access_token().await.unwrap(), "tok-123");
        // Second call serves from cache (token not near expiry).
        assert_eq!(tp.get_access_token().await.unwrap(), "tok-123");
    }

    #[tokio::test]
    async fn refreshes_after_invalidate() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "tok-A",
                "expires_in": 3600
            })))
            .expect(2)
            .mount(&server)
            .await;

        let tp = TokenProvider::new(TokenProviderOptions::new(server.uri(), "cid", "sk_secret"))
            .unwrap();
        assert_eq!(tp.get_access_token().await.unwrap(), "tok-A");
        tp.invalidate().await;
        assert_eq!(tp.get_access_token().await.unwrap(), "tok-A");
    }

    #[tokio::test]
    async fn surfaces_http_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
            .mount(&server)
            .await;

        let tp = TokenProvider::new(TokenProviderOptions::new(server.uri(), "cid", "bad")).unwrap();
        let err = tp.get_access_token().await.unwrap_err();
        match err {
            TokenError::Status { status, .. } => assert_eq!(status, 401),
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn empty_config_rejected() {
        assert!(TokenProvider::new(TokenProviderOptions::new("", "c", "s")).is_err());
        assert!(TokenProvider::new(TokenProviderOptions::new("u", "", "s")).is_err());
        assert!(TokenProvider::new(TokenProviderOptions::new("u", "c", "")).is_err());
    }
}
