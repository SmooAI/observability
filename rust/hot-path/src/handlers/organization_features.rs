//! GET /v1/organizations/:org_id/features — port of
//! `packages/backend/src/routes/organization-features.ts` + the
//! `getOrganizationFeatures` helper in
//! `packages/db/src/services/retail/product-access-service.ts`.
//!
//! Precedence (highest → lowest), mirroring SMOODEV-1014:
//!   1. Per-org override row (active, non-expired): `enabled=true` adds,
//!      `enabled=false` removes — trumps EVERY downstream layer.
//!   2. `DEFAULT_ENABLED_FEATURES` — always-on for every org.
//!   3. `INTERNAL_ONLY_FEATURES` + `@smoo.ai` requesting user.
//!   4. Active Stripe products mapped to features via `PRODUCT_FEATURE_MAP`
//!      and `stripe_products.metadata.features`.
//!
//! Auth: the JWT-decoded `sub` must be a member of `org_id`. If not → 403.
//! The TS route relies on the standard auth middleware + RLS to enforce
//! this; we enforce it explicitly in SQL since we use the admin pool.
//!
//! Caching: read-through with key `features:{org_id}:{user_id}` and TTL
//! 60s. The (org, user) tuple is important — internal-segment expansion
//! makes the response user-dependent, so caching by `org_id` alone would
//! leak `INTERNAL_ONLY_FEATURES` to public users.
//!
//! Response shape: `{ organizationId: string, features: string[] }` —
//! features are sorted for stable JSON, which matters for the
//! canonical-JSON hash the shadow harness uses to compare against the TS
//! response.

use std::collections::HashSet;

use axum::{
    extract::{Path, State},
    http::header::{HeaderMap, AUTHORIZATION},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::jwt::extract_bearer;
use crate::cache;
use crate::error::AppError;
use crate::product_constants::{expand_with_internal, features_for_product, DEFAULT_ENABLED_FEATURES};
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct OrganizationFeaturesResponse {
    #[serde(rename = "organizationId")]
    pub organization_id: String,
    /// Sorted ascending for stable JSON. Matches the canonical-JSON
    /// stableStringify convention used by `shadow-mirror.ts`.
    pub features: Vec<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ProductFeatureRow {
    product_name: String,
    metadata: Option<Value>,
}

#[derive(Debug, sqlx::FromRow)]
struct OverrideRow {
    feature_key: String,
    enabled: bool,
}

pub async fn get_organization_features(
    Path(org_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OrganizationFeaturesResponse>, AppError> {
    let auth_header = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let token = extract_bearer(auth_header)?;

    let claims = state.jwks.verify(token).await?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized("invalid user id in JWT".to_string()))?;

    // Membership check up front: must be a member of the org. The TS
    // route relies on RLS for this; we use the admin pool so we have to
    // enforce it explicitly. A super-admin path (admin_user_roles) could
    // bypass — out of scope for the hot-path read endpoint; the slow path
    // on Lambda still handles those callers.
    let is_member: Option<i32> = sqlx::query_scalar(
        r#"
        SELECT 1 FROM organization_members
        WHERE organization_id = $1 AND user_id = $2
        LIMIT 1
        "#,
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;

    if is_member.is_none() {
        return Err(AppError::Forbidden(
            "Not a member of this organization".to_string(),
        ));
    }

    let cache_key = cache::organization_features_key(&org_id.to_string(), &user_id.to_string());

    // Read-through cache — best-effort.
    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(Some(cached)) = cache::get_string(&mut conn, &cache_key).await {
            if let Ok(parsed) = serde_json::from_str::<OrganizationFeaturesResponse>(&cached) {
                return Ok(Json(parsed));
            }
        }
    }

    let email = claims.email.as_deref();

    let features = compute_features(&state, org_id, email).await?;

    let mut sorted: Vec<String> = features.into_iter().collect();
    sorted.sort();

    let body = OrganizationFeaturesResponse {
        organization_id: org_id.to_string(),
        features: sorted,
    };

    // Best-effort cache write — 60s TTL.
    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(json) = serde_json::to_string(&body) {
            let _ = cache::set_string(&mut conn, &cache_key, &json, cache::SIDEBAR_TTL_SECS).await;
        }
    }

    Ok(Json(body))
}

async fn compute_features(
    state: &AppState,
    org_id: Uuid,
    email: Option<&str>,
) -> Result<HashSet<String>, AppError> {
    // Fire both DB reads in parallel. Matches the TS service which does
    // the same — overrides are usually empty so the second await is
    // effectively free.
    let (overrides_result, products_result) = tokio::join!(
        sqlx::query_as::<_, OverrideRow>(
            r#"
            SELECT feature_key, enabled
            FROM product_access_overrides
            WHERE organization_id = $1
              AND (expires_at IS NULL OR expires_at > NOW())
            "#,
        )
        .bind(org_id)
        .fetch_all(&state.pool),
        sqlx::query_as::<_, ProductFeatureRow>(
            r#"
            SELECT sp.name AS product_name, sp.metadata
            FROM products p
            INNER JOIN stripe_products sp ON p.stripe_product_id = sp.id
            WHERE p.organization_id = $1
              AND p.status = 'active'
            "#,
        )
        .bind(org_id)
        .fetch_all(&state.pool),
    );

    let overrides = overrides_result?;
    let products = products_result?;

    let mut features: HashSet<String> = DEFAULT_ENABLED_FEATURES.iter().map(|s| (*s).to_string()).collect();

    for row in &products {
        for f in features_for_product(&row.product_name) {
            features.insert((*f).to_string());
        }

        // metadata.features array — forward-compat path used for orgs
        // granted a feature via Stripe metadata before PRODUCT_FEATURE_MAP
        // is updated (e.g. signal_agents → Chakra). The TS code parses
        // `metadata.features` as a JSON-encoded string of an array; mirror
        // that exactly.
        if let Some(metadata) = &row.metadata {
            if let Some(features_str) = metadata.get("features").and_then(|v| v.as_str()) {
                if let Ok(parsed) = serde_json::from_str::<Vec<String>>(features_str) {
                    for f in parsed {
                        features.insert(f);
                    }
                }
            }
        }
    }

    expand_with_internal(&mut features, email);

    // Overrides apply LAST — trump every other layer.
    for ov in overrides {
        if ov.enabled {
            features.insert(ov.feature_key);
        } else {
            features.remove(&ov.feature_key);
        }
    }

    Ok(features)
}
