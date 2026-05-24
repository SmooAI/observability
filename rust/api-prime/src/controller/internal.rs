//! Internal cluster-only endpoints. The only consumer today is mutation
//! Lambdas calling `POST /internal/v1/cache/invalidate` over the in-VPC
//! ClusterIP service. See ADR-017 §"Cache invalidation".
//!
//! Auth is HMAC-SHA256 of the raw request body, signed with the secret
//! `apiPrimeEdgeAttestSecret` (provisioned via `@smooai/config` →
//! ExternalSecret → env `EDGE_ATTEST_SECRET`). The signature is sent in
//! the header `X-Smoo-Invalidate-Sig` as lowercase hex.
//!
//! On accept the controller:
//!   1. Resolves each tag → cache key set via `apr:tag:<tag>`.
//!   2. DELs each matching cache entry + the tag set itself.
//!   3. PUBLISHes the tags on `apr:invalidate`.
//!   4. Returns 204.
//!
//! ## Tag → cache-key reverse index contract
//!
//! The **data plane** is responsible for maintaining `apr:tag:<tag>` Redis
//! SETs whenever it writes a cache entry to `apr:cache:<key>`. Specifically,
//! when the data plane caches a response under `apr:cache:<key>` with tags
//! `["org:xyz", "user:abc"]`, it MUST also `SADD apr:tag:org:xyz <key>` and
//! `SADD apr:tag:user:abc <key>` in the same Lua script or pipeline. The
//! controller relies on this to enumerate cache keys during invalidation —
//! it does NOT scan `apr:cache:*` (too expensive). See
//! `docs/Architecture/API-Prime.md` for the full contract.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use hmac::{Hmac, Mac};
use redis::AsyncCommands;
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::controller::pubsub;

type HmacSha256 = Hmac<Sha256>;

pub const HEADER_INVALIDATE_SIG: &str = "x-smoo-invalidate-sig";

/// Shared state for the internal/admin endpoints.
#[derive(Clone)]
pub struct ControllerState {
    pub redis: redis::Client,
    pub edge_attest_secret: Arc<String>,
    pub snapshot: Arc<tokio::sync::RwLock<crate::controller::reconcile::ReconcileSnapshot>>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Header treated as proof of admin auth in Phase 1. Phase 2 swaps this
    /// for real JWT verification (see `admin::require_admin`).
    pub admin_dev_bypass: bool,
}

#[derive(Debug, Deserialize)]
pub struct InvalidateRequest {
    pub tags: Vec<String>,
}

/// `POST /internal/v1/cache/invalidate`.
pub async fn invalidate(
    State(state): State<ControllerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1. Pull signature header
    let sig_header = match headers.get(HEADER_INVALIDATE_SIG) {
        Some(h) => h,
        None => {
            tracing::warn!("invalidate request missing signature header");
            return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
        }
    };
    let sig_hex = match sig_header.to_str() {
        Ok(s) => s,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid signature encoding").into_response(),
    };

    // 2. Verify HMAC over the raw body
    if !verify_hmac(state.edge_attest_secret.as_bytes(), &body, sig_hex) {
        tracing::warn!(
            sig_len = sig_hex.len(),
            body_len = body.len(),
            "invalidate request HMAC verification failed (possible compromise indicator)"
        );
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    // 3. Parse body
    let req: InvalidateRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(error = %err, "invalidate request body parse failed");
            return (StatusCode::BAD_REQUEST, format!("invalid body: {}", err)).into_response();
        }
    };

    if req.tags.is_empty() {
        return (StatusCode::BAD_REQUEST, "tags must be non-empty").into_response();
    }

    // 4. Apply invalidation
    match perform_invalidation(&state, &req.tags).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "invalidate failed: redis or pubsub unreachable");
            (StatusCode::SERVICE_UNAVAILABLE, "valkey unreachable").into_response()
        }
    }
}

/// Core invalidation logic — shared with the admin
/// `POST /admin/v1/routes/:id/cache/invalidate` endpoint.
pub async fn perform_invalidation(state: &ControllerState, tags: &[String]) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;

    for tag in tags {
        let tag_key = format!("apr:tag:{}", tag);
        // SMEMBERS = enumerate the cache keys this tag points to. Data
        // plane is responsible for populating these sets on write; see
        // the "Tag → cache-key reverse index" contract at the top of this
        // module and docs/Architecture/API-Prime.md.
        let keys: Vec<String> = conn.smembers(&tag_key).await.unwrap_or_default();
        if !keys.is_empty() {
            let _: () = conn.del(&keys).await?;
        }
        let _: () = conn.del(&tag_key).await?;
    }

    pubsub::publish_invalidation(&mut conn, tags).await?;
    Ok(())
}

/// Constant-time HMAC-SHA256 verification over hex-encoded signature.
pub fn verify_hmac(secret: &[u8], body: &[u8], sig_hex: &str) -> bool {
    let expected = match hex::decode(sig_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let computed = mac.finalize().into_bytes();

    // ct compare; the `verify_slice` path would also work, but using
    // `subtle::ConstantTimeEq` directly is more explicit about intent and
    // independent of the hmac crate's internal MAC type.
    computed.ct_eq(&expected).into()
}

/// Compute the HMAC hex of a body. Exposed for tests + admin-side
/// invalidations that don't traverse the HTTP boundary but should still
/// produce the same signature for downstream consumers.
pub fn sign_body(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Trivial wrapper so the bin can wire this with `axum::Json` for the
/// admin shadow endpoint.
pub async fn admin_invalidate(
    State(state): State<ControllerState>,
    Json(req): Json<InvalidateRequest>,
) -> Response {
    if req.tags.is_empty() {
        return (StatusCode::BAD_REQUEST, "tags must be non-empty").into_response();
    }
    match perform_invalidation(&state, &req.tags).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "admin invalidate failed");
            (StatusCode::SERVICE_UNAVAILABLE, "valkey unreachable").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_hmac_happy_path() {
        let secret = b"hunter2";
        let body = br#"{"tags":["org:xyz"]}"#;
        let sig = sign_body(secret, body);
        assert!(verify_hmac(secret, body, &sig));
    }

    #[test]
    fn verify_hmac_rejects_wrong_body() {
        let secret = b"hunter2";
        let sig = sign_body(secret, b"original");
        assert!(!verify_hmac(secret, b"tampered", &sig));
    }

    #[test]
    fn verify_hmac_rejects_wrong_secret() {
        let sig = sign_body(b"real", b"body");
        assert!(!verify_hmac(b"fake", b"body", &sig));
    }

    #[test]
    fn verify_hmac_rejects_non_hex_signature() {
        assert!(!verify_hmac(b"k", b"body", "not-hex-!@#"));
    }

    #[test]
    fn verify_hmac_rejects_truncated_signature() {
        let sig = sign_body(b"k", b"body");
        let truncated = &sig[..sig.len() - 2];
        assert!(!verify_hmac(b"k", b"body", truncated));
    }
}
