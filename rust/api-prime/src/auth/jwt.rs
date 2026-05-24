//! Supabase JWT verification.
//!
//! This is the Rust port of the verification path in
//! `packages/auth/src/server/verify.ts` (smooai monorepo). We diverge in
//! one important way: we do NOT call `supabase.auth.getUser()` to fetch
//! `app_metadata.orgId`. Instead, we rely on the decoded JWT claims
//! (`sub`, `email`, `aud`). If org-scoping is needed in a future endpoint,
//! query `organization_members` by `user_id` directly via sqlx. This
//! eliminates an extra ~50ms HTTP roundtrip per request — the whole
//! reason this service exists.
//!
//! JWKS handling:
//! - JWKS document is fetched from `SUPABASE_JWKS_URL`.
//! - The set is cached in memory with a 1-hour TTL.
//! - On a `kid` miss we force a refresh once before failing.
//! - We support ES256 (asymmetric, the Supabase default for v2 projects)
//!   only. HS256 verification would need the shared secret to be wired in
//!   via `@smooai/config` and is intentionally out of scope for this PR.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::AppError;

const JWKS_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// User UUID (Supabase auth.users.id).
    pub sub: String,
    pub aud: String,
    pub exp: usize,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub iss: Option<String>,
}

#[derive(Clone)]
pub struct JwksCache {
    inner: Arc<RwLock<JwksState>>,
    url: String,
}

struct JwksState {
    set: Option<JwkSet>,
    fetched_at: Option<Instant>,
}

impl JwksCache {
    pub fn new(url: String) -> Self {
        Self {
            inner: Arc::new(RwLock::new(JwksState {
                set: None,
                fetched_at: None,
            })),
            url,
        }
    }

    async fn refresh(&self) -> Result<(), AppError> {
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?
            .get(&self.url)
            .send()
            .await?
            .error_for_status()?;
        let set: JwkSet = response.json().await?;
        let mut guard = self.inner.write().await;
        guard.set = Some(set);
        guard.fetched_at = Some(Instant::now());
        Ok(())
    }

    async fn current(&self, force_refresh: bool) -> Result<JwkSet, AppError> {
        if !force_refresh {
            let guard = self.inner.read().await;
            if let (Some(set), Some(at)) = (&guard.set, guard.fetched_at) {
                if at.elapsed() < JWKS_TTL {
                    return Ok(set.clone());
                }
            }
        }
        drop(self.inner.read().await);
        self.refresh().await?;
        let guard = self.inner.read().await;
        guard
            .set
            .clone()
            .ok_or_else(|| AppError::Unauthorized("JWKS unavailable".to_string()))
    }

    /// Decode + verify a Supabase JWT. Returns the parsed claims on success.
    pub async fn verify(&self, token: &str) -> Result<Claims, AppError> {
        let header = decode_header(token).map_err(|e| AppError::Unauthorized(format!("invalid JWT header: {e}")))?;
        let kid = header
            .kid
            .ok_or_else(|| AppError::Unauthorized("JWT header missing kid".to_string()))?;

        // First try with the cached JWKS, then on miss force a refresh
        // once before giving up.
        let mut force = false;
        for _ in 0..2 {
            let set = self.current(force).await?;
            if let Some(jwk) = set.find(&kid) {
                let key = DecodingKey::from_jwk(jwk)
                    .map_err(|e| AppError::Unauthorized(format!("invalid JWK: {e}")))?;
                let alg = match header.alg {
                    Algorithm::ES256 => Algorithm::ES256,
                    other => {
                        return Err(AppError::Unauthorized(format!(
                            "unsupported JWT algorithm: {other:?} (this service supports ES256 only)"
                        )))
                    }
                };
                let mut validation = Validation::new(alg);
                validation.set_audience(&["authenticated"]);
                let data = decode::<Claims>(token, &key, &validation)
                    .map_err(|e| AppError::Unauthorized(format!("JWT verification failed: {e}")))?;
                return Ok(data.claims);
            }
            force = true;
        }

        Err(AppError::Unauthorized(format!("no JWK matched kid={}", kid)))
    }
}

/// Extract the bearer token from a typical `Authorization: Bearer <token>`
/// header value.
pub fn extract_bearer(authorization: Option<&str>) -> Result<&str, AppError> {
    let header = authorization.ok_or_else(|| AppError::Unauthorized("missing Authorization header".to_string()))?;
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .ok_or_else(|| AppError::Unauthorized("malformed Authorization header".to_string()))?;
    if token.is_empty() {
        return Err(AppError::Unauthorized("empty bearer token".to_string()));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_extraction() {
        assert_eq!(extract_bearer(Some("Bearer abc")).unwrap(), "abc");
        assert_eq!(extract_bearer(Some("bearer xyz")).unwrap(), "xyz");
        assert!(extract_bearer(None).is_err());
        assert!(extract_bearer(Some("Basic foo")).is_err());
        assert!(extract_bearer(Some("Bearer ")).is_err());
    }
}
