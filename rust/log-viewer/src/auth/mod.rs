//! M2M `client_credentials` auth manager.
//!
//! Exchanges a per-org `client_id` / `client_secret` (stored in the OS
//! keychain) for a bearer JWT against `https://auth.smoo.ai/token`, caches the
//! token in memory, and refreshes 60s before expiry. See
//! `docs/Engineering/Rust-Desktop-Observability-Viewer.md` §5.3 for the
//! decision rationale.
//!
//! The cache is intentionally **never persisted to disk** — only the
//! credentials live in the OS keychain. On app restart, the manager performs a
//! fresh `client_credentials` exchange the first time any view needs a bearer.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

pub mod keychain;
pub mod oauth;

#[allow(unused_imports)]
pub use keychain::{Credentials, Keychain};
#[allow(unused_imports)]
pub use oauth::OAuthClient;

pub const TOKEN_URL: &str = "https://auth.smoo.ai/token";
pub const API_BASE: &str = "https://api.smoo.ai";

/// Refresh `REFRESH_LEEWAY` before the access_token's `expires_in` so a token
/// returned right at the threshold doesn't get used past expiry.
const REFRESH_LEEWAY: Duration = Duration::seconds(60);

#[derive(Debug, Deserialize, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credentials stored for org {0}")]
    MissingCredentials(Uuid),
    #[error("token endpoint returned status {0}")]
    TokenEndpoint(reqwest::StatusCode),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
}

#[derive(Clone, Debug)]
struct CachedToken {
    access_token: String,
    expires_at: DateTime<Utc>,
}

impl CachedToken {
    fn fresh(&self) -> bool {
        self.expires_at > Utc::now() + REFRESH_LEEWAY
    }
}

/// Owns the keychain handle, the cache, and an `OAuthClient`. Thread-safe and
/// cheap to clone (internals share a `Mutex` for the cache map).
#[derive(Clone)]
pub struct AuthManager {
    inner: std::sync::Arc<AuthInner>,
}

struct AuthInner {
    keychain: Keychain,
    oauth: OAuthClient,
    cache: Mutex<HashMap<Uuid, CachedToken>>,
    /// In-memory credential overrides. Consulted *before* the OS keychain. Used
    /// by the Settings "Add Org" wizard to verify creds before committing them
    /// to the keychain, and by tests to avoid hosted-runner keychain calls.
    overrides: Mutex<HashMap<Uuid, Credentials>>,
}

impl AuthManager {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            inner: std::sync::Arc::new(AuthInner {
                keychain: Keychain::new(),
                oauth: OAuthClient::new(http),
                cache: Mutex::new(HashMap::new()),
                overrides: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Test/staging helper — override the token URL.
    pub fn with_token_url(self, url: impl Into<String>) -> Self {
        let inner = AuthInner {
            keychain: self.inner.keychain,
            oauth: self.inner.oauth.clone().with_token_url(url),
            cache: Mutex::new(std::mem::take(&mut *self.inner.cache.lock().unwrap())),
            overrides: Mutex::new(std::mem::take(
                &mut *self.inner.overrides.lock().unwrap(),
            )),
        };
        Self {
            inner: std::sync::Arc::new(inner),
        }
    }

    /// Stash a credential pair in memory, bypassing the keychain on the next
    /// `bearer_for(org)` call. Used by the Settings "Add Org" wizard to verify
    /// candidate creds before writing them to the keychain, and by tests.
    pub fn set_override(&self, org: Uuid, client_id: &str, client_secret: &str) {
        self.inner
            .overrides
            .lock()
            .expect("auth overrides poisoned")
            .insert(
                org,
                Credentials {
                    client_id: client_id.to_string(),
                    client_secret: client_secret.to_string(),
                },
            );
    }

    pub fn clear_override(&self, org: Uuid) {
        self.inner
            .overrides
            .lock()
            .expect("auth overrides poisoned")
            .remove(&org);
    }

    pub fn keychain(&self) -> Keychain {
        self.inner.keychain
    }

    /// Return a bearer token for `org`. Uses the cache when possible; otherwise
    /// reads creds from in-memory overrides (if set) or the OS keychain, then
    /// exchanges them at the token endpoint.
    pub async fn bearer_for(&self, org: Uuid) -> Result<String, AuthError> {
        if let Some(token) = self.cached_bearer(org) {
            return Ok(token);
        }
        let creds = self.creds_for(org)?;
        let resp = self
            .inner
            .oauth
            .exchange(&creds.client_id, &creds.client_secret)
            .await?;
        let cached = CachedToken {
            access_token: resp.access_token.clone(),
            expires_at: Utc::now() + Duration::seconds(resp.expires_in),
        };
        self.inner
            .cache
            .lock()
            .expect("auth cache poisoned")
            .insert(org, cached);
        Ok(resp.access_token)
    }

    /// Force a refresh on the next `bearer_for` call (used after a 401 from an
    /// API call).
    pub fn invalidate(&self, org: Uuid) {
        self.inner
            .cache
            .lock()
            .expect("auth cache poisoned")
            .remove(&org);
    }

    /// Test/verify — exchange tokens without caching. Used by the Settings
    /// "Verify" button so a temporary credential is not silently cached.
    pub async fn verify(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, AuthError> {
        self.inner.oauth.exchange(client_id, client_secret).await
    }

    fn cached_bearer(&self, org: Uuid) -> Option<String> {
        let cache = self.inner.cache.lock().expect("auth cache poisoned");
        cache
            .get(&org)
            .filter(|t| t.fresh())
            .map(|t| t.access_token.clone())
    }

    /// Returns the credentials for `org`, preferring the in-memory override
    /// over the keychain.
    fn creds_for(&self, org: Uuid) -> Result<Credentials, AuthError> {
        if let Some(creds) = self
            .inner
            .overrides
            .lock()
            .expect("auth overrides poisoned")
            .get(&org)
            .cloned()
        {
            return Ok(creds);
        }
        self.inner.keychain.get(org)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_token_freshness_uses_leeway() {
        let too_close = CachedToken {
            access_token: "abc".into(),
            expires_at: Utc::now() + Duration::seconds(30),
        };
        assert!(!too_close.fresh(), "token expiring within leeway is stale");

        let fresh = CachedToken {
            access_token: "abc".into(),
            expires_at: Utc::now() + Duration::seconds(3600),
        };
        assert!(fresh.fresh());
    }
}
