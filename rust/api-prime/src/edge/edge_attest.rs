//! Edge attestation — the trust boundary between api-prime and the
//! Lambdas behind it.
//!
//! ADR-017 §"Trust boundary": the data plane verified the JWT, enforced
//! the rate limit, and ran schema-ingress. We tell the Lambda what we
//! enforced via an HMAC-signed claim set; the Lambda checks the HMAC
//! and skips its own auth / rate-limit / schema-ingress if present +
//! fresh (≤30s old). Shared secret is delivered via @smooai/config
//! under `EDGE_ATTEST_SECRET`.
//!
//! Original draft put this in an HTTP header; the architectural pivot
//! to direct Lambda InvokeFunction (also ADR-017) means we no longer
//! have a header — instead we embed the payload into the API Gateway
//! proxy event under `requestContext.authorizer.smooEdge`. Sirius's
//! Hono middleware reads that field and verifies.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::edge::auth::EdgeAuthContext;
use crate::edge::types::RouteEntry;

type HmacSha256 = Hmac<Sha256>;

/// Claims embedded in the Lambda payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeAttestClaims {
    /// Issuer — always `"api-prime"` when we sign.
    pub iss: String,
    pub iat: u64,
    pub exp: u64,
    pub sub: String,
    /// `"user"` / `"m2m"` / `"public"`.
    pub kind: String,
    pub route: String,
    pub enforced: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeAttestPayload {
    pub claims: EdgeAttestClaims,
    pub sig: String,
}

#[derive(Clone)]
pub struct EdgeAttestSigner {
    secret: Vec<u8>,
}

impl EdgeAttestSigner {
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        Self { secret: secret.into() }
    }

    pub fn sign(&self, route: &RouteEntry, auth: &EdgeAuthContext) -> EdgeAttestPayload {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let claims = EdgeAttestClaims {
            iss: "api-prime".to_string(),
            iat: now,
            exp: now + 30,
            sub: auth.sub.clone(),
            kind: match auth.kind {
                crate::edge::auth::AuthKind::User { .. } => "user",
                crate::edge::auth::AuthKind::M2m { .. } => "m2m",
                crate::edge::auth::AuthKind::Public => "public",
            }
            .to_string(),
            route: format!("{} {}", route.method.to_uppercase(), route.path),
            enforced: vec!["auth".to_string(), "rate-limit".to_string(), "schema-ingress".to_string()],
        };
        let sig = self.sign_claims(&claims);
        EdgeAttestPayload { claims, sig }
    }

    /// Verify an attestation payload using this signer's secret.
    pub fn verify(&self, payload: &EdgeAttestPayload) -> bool {
        match hex::decode(&payload.sig) {
            Ok(bytes) => {
                let mut mac = match HmacSha256::new_from_slice(&self.secret) {
                    Ok(m) => m,
                    Err(_) => return false,
                };
                let body = match canonical_bytes(&payload.claims) {
                    Ok(b) => b,
                    Err(_) => return false,
                };
                mac.update(&body);
                mac.verify_slice(&bytes).is_ok()
            }
            Err(_) => false,
        }
    }

    fn sign_claims(&self, claims: &EdgeAttestClaims) -> String {
        let body = canonical_bytes(claims).expect("claims serialize is infallible");
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("hmac accepts any key length");
        mac.update(&body);
        hex::encode(mac.finalize().into_bytes())
    }
}

/// Canonical serialization used for signing.
fn canonical_bytes(claims: &EdgeAttestClaims) -> Result<Vec<u8>, serde_json::Error> {
    let raw = serde_json::to_vec(claims)?;
    Ok(URL_SAFE_NO_PAD.encode(&raw).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::auth::AuthKind;
    use crate::edge::types::{AuthRequirement, RateLimitConfig, RouteMode};

    fn route() -> RouteEntry {
        RouteEntry {
            path: "/foo/:id".into(),
            method: "GET".into(),
            auth: AuthRequirement::User,
            idempotent: true,
            mode: RouteMode::Proxy,
            rate_limit: RateLimitConfig {
                per_token: 100,
                window_seconds: 60,
            },
            cache: None,
            implement: None,
            lambda_arn: Some("arn".into()),
            schema_ref: None,
        }
    }

    fn auth() -> EdgeAuthContext {
        EdgeAuthContext {
            sub: "user-abc".into(),
            kind: AuthKind::User { user_id: "user-abc".into() },
            raw_jwt: None,
        }
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let signer = EdgeAttestSigner::new(*b"shared-secret-32-bytes-padding..");
        let payload = signer.sign(&route(), &auth());
        assert_eq!(payload.claims.iss, "api-prime");
        assert_eq!(payload.claims.sub, "user-abc");
        assert_eq!(payload.claims.kind, "user");
        assert!(payload.claims.exp > payload.claims.iat);
        assert!(signer.verify(&payload));
    }

    #[test]
    fn tampering_breaks_verification() {
        let signer = EdgeAttestSigner::new(*b"k");
        let mut payload = signer.sign(&route(), &auth());
        payload.claims.sub = "other-user".into();
        assert!(!signer.verify(&payload));
    }

    #[test]
    fn different_secrets_dont_verify() {
        let signer_a = EdgeAttestSigner::new(*b"secret-a");
        let signer_b = EdgeAttestSigner::new(*b"secret-b");
        let payload = signer_a.sign(&route(), &auth());
        assert!(!signer_b.verify(&payload));
    }
}
