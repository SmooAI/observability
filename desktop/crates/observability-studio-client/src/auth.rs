//! M2M `client_credentials` against `https://auth.smoo.ai/token`.
//!
//! - Credentials live in the OS keychain (`keyring`), keyed by org UUID.
//! - Access tokens are cached in memory, never persisted, refreshed 60s
//!   before expiry.
//! - `verify()` bypasses the cache so the Settings → Add Org flow doesn't
//!   pollute cache state on a typo.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

pub const TOKEN_URL: &str = "https://auth.smoo.ai/token";
pub const API_BASE: &str = "https://api.smoo.ai";
const KEYCHAIN_SERVICE: &str = "ai.smoo.observability.studio";

/// Refresh `REFRESH_LEEWAY` before `expires_in` so a token returned right at
/// the boundary doesn't get used past expiry.
const REFRESH_LEEWAY: Duration = Duration::seconds(60);

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
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

#[derive(Clone)]
pub struct AuthManager {
    inner: std::sync::Arc<AuthInner>,
}

struct AuthInner {
    http: reqwest::Client,
    cache: Mutex<HashMap<Uuid, CachedToken>>,
    token_url: String,
}

impl AuthManager {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            inner: std::sync::Arc::new(AuthInner {
                http,
                cache: Mutex::new(HashMap::new()),
                token_url: TOKEN_URL.to_string(),
            }),
        }
    }

    /// Override the token URL for tests / staging.
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        // We're the sole owner here; `try_unwrap` succeeds.
        let inner = std::sync::Arc::get_mut(&mut self.inner);
        if let Some(inner) = inner {
            inner.token_url = url.into();
        } else {
            // Cloned elsewhere — rebuild the Arc.
            let new_inner = AuthInner {
                http: self.inner.http.clone(),
                cache: Mutex::new(std::mem::take(
                    &mut *self.inner.cache.lock().expect("auth cache poisoned"),
                )),
                token_url: url.into(),
            };
            self.inner = std::sync::Arc::new(new_inner);
        }
        self
    }

    /// Exchange `client_credentials` without caching the resulting token —
    /// used by the verify-before-store path in Settings → Add Org.
    pub async fn verify(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, AuthError> {
        self.exchange(client_id, client_secret).await
    }

    /// Return a fresh bearer for `org`. Hits the cache first; otherwise reads
    /// from the OS keychain and exchanges at `auth.smoo.ai/token`.
    pub async fn bearer_for(&self, org: Uuid) -> Result<String, AuthError> {
        if let Some(t) = self.cached_bearer(org) {
            return Ok(t);
        }
        let (cid, csec) = read_credentials(org)?;
        let resp = self.exchange(&cid, &csec).await?;
        self.inner
            .cache
            .lock()
            .expect("auth cache poisoned")
            .insert(
                org,
                CachedToken {
                    access_token: resp.access_token.clone(),
                    expires_at: Utc::now() + Duration::seconds(resp.expires_in),
                },
            );
        Ok(resp.access_token)
    }

    /// Drop the cached token for `org` so the next `bearer_for` re-mints.
    pub fn invalidate(&self, org: Uuid) {
        self.inner
            .cache
            .lock()
            .expect("auth cache poisoned")
            .remove(&org);
    }

    async fn exchange(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, AuthError> {
        let resp = self
            .inner
            .http
            .post(&self.inner.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("provider", "client_credentials"),
                ("client_id", client_id),
                ("client_secret", client_secret),
            ])
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let _ = resp.text().await; // drain
            return Err(AuthError::TokenEndpoint(status));
        }
        Ok(resp.json().await?)
    }

    fn cached_bearer(&self, org: Uuid) -> Option<String> {
        let cache = self.inner.cache.lock().expect("auth cache poisoned");
        cache
            .get(&org)
            .filter(|t| t.fresh())
            .map(|t| t.access_token.clone())
    }
}

pub fn store_credentials(
    org: Uuid,
    client_id: &str,
    client_secret: &str,
) -> Result<(), AuthError> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &account(org, "client_id"))?
        .set_password(client_id)?;
    keyring::Entry::new(KEYCHAIN_SERVICE, &account(org, "client_secret"))?
        .set_password(client_secret)?;
    Ok(())
}

pub fn read_credentials(org: Uuid) -> Result<(String, String), AuthError> {
    let cid = keyring::Entry::new(KEYCHAIN_SERVICE, &account(org, "client_id"))?
        .get_password()
        .map_err(|e| match e {
            keyring::Error::NoEntry => AuthError::MissingCredentials(org),
            other => other.into(),
        })?;
    let sec = keyring::Entry::new(KEYCHAIN_SERVICE, &account(org, "client_secret"))?
        .get_password()
        .map_err(|e| match e {
            keyring::Error::NoEntry => AuthError::MissingCredentials(org),
            other => other.into(),
        })?;
    Ok((cid, sec))
}

pub fn remove_credentials(org: Uuid) -> Result<(), AuthError> {
    let _ = keyring::Entry::new(KEYCHAIN_SERVICE, &account(org, "client_id"))?
        .delete_credential();
    let _ = keyring::Entry::new(KEYCHAIN_SERVICE, &account(org, "client_secret"))?
        .delete_credential();
    Ok(())
}

fn account(org: Uuid, field: &str) -> String {
    format!("{org}::{field}")
}
