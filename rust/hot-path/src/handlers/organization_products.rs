//! GET /v1/organizations/:org_id/products — port of the GET handler in
//! `packages/backend/src/routes/products.ts`.
//!
//! Auth + scoping: caller's JWT must be a member of `org_id`. The TS
//! route does this via RLS; we enforce with an explicit membership check.
//!
//! Response shape: the TS handler returns `tx.query.products.findMany`
//! with `order` and `stripeProduct` embedded relations. We replicate that
//! using a LEFT JOIN and `json_build_object` so the shape matches what
//! the dashboard already consumes.
//!
//! Caching: read-through with key `products:{org_id}:{user_id}` and TTL
//! 60s. We key on the user too even though product rows are org-scoped —
//! a per-user key keeps the invalidation strategy uniform across the
//! three sidebar reads and avoids a cross-user cache stampede when one
//! user's view forces a refresh.

use std::str::FromStr;

use axum::{
    extract::{Path, State},
    http::header::{HeaderMap, AUTHORIZATION},
    Json,
};
use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::jwt::extract_bearer;
use crate::cache;
use crate::error::AppError;
use crate::state::AppState;

/// Mirrors the `products.findMany({ with: { order: true, stripeProduct: true } })`
/// shape from Drizzle.
///
/// Timestamps are `timestamp` (NOT `timestamp with timezone`) in the DB
/// for the retail tables (see `packages/db/src/schemas/retail/products.ts`).
/// We deserialize to `NaiveDateTime` and re-serialize with a `Z` suffix so
/// the JSON matches the TS Drizzle output (`new Date().toISOString()`).
#[derive(Debug, Serialize, Deserialize)]
pub struct ProductWithRelations {
    pub id: Uuid,
    #[serde(rename = "organizationId")]
    pub organization_id: Uuid,
    #[serde(rename = "purchasedBy")]
    pub purchased_by: Uuid,
    #[serde(rename = "orderId")]
    pub order_id: Uuid,
    #[serde(rename = "stripeProductId")]
    pub stripe_product_id: Uuid,
    pub status: String,
    #[serde(rename = "trialEndsAt")]
    pub trial_ends_at: Option<DateTime<chrono::Utc>>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<chrono::Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<chrono::Utc>,
    /// `null` if the order was deleted (orders.organizationId is ON DELETE SET NULL).
    pub order: Option<Value>,
    #[serde(rename = "stripeProduct")]
    pub stripe_product: Option<Value>,
}

#[derive(Debug, sqlx::FromRow)]
struct RawProductRow {
    id: Uuid,
    organization_id: Uuid,
    purchased_by: Uuid,
    order_id: Uuid,
    stripe_product_id: Uuid,
    status: String,
    trial_ends_at: Option<NaiveDateTime>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
    order: Option<Value>,
    stripe_product: Option<Value>,
}

fn to_utc(ts: NaiveDateTime) -> DateTime<chrono::Utc> {
    DateTime::<chrono::Utc>::from_naive_utc_and_offset(ts, chrono::Utc)
}

impl From<RawProductRow> for ProductWithRelations {
    fn from(row: RawProductRow) -> Self {
        Self {
            id: row.id,
            organization_id: row.organization_id,
            purchased_by: row.purchased_by,
            order_id: row.order_id,
            stripe_product_id: row.stripe_product_id,
            status: row.status,
            trial_ends_at: row.trial_ends_at.map(to_utc),
            created_at: to_utc(row.created_at),
            updated_at: to_utc(row.updated_at),
            order: row.order,
            stripe_product: row.stripe_product,
        }
    }
}

pub async fn list_organization_products(
    Path(org_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProductWithRelations>>, AppError> {
    let auth_header = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let token = extract_bearer(auth_header)?;

    let claims = state.jwks.verify(token).await?;
    let user_id = Uuid::from_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized("invalid user id in JWT".to_string()))?;

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

    let cache_key = cache::organization_products_key(&org_id.to_string(), &user_id.to_string());

    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(Some(cached)) = cache::get_string(&mut conn, &cache_key).await {
            if let Ok(parsed) = serde_json::from_str::<Vec<ProductWithRelations>>(&cached) {
                return Ok(Json(parsed));
            }
        }
    }

    // Drizzle's `findMany({ with: { order: true, stripeProduct: true } })`
    // emits a query like this — embed the related rows as JSON so axum can
    // pass them through verbatim. Field names use camelCase to match the
    // TS `selectOrderSchema` / `selectStripeProductSchema` output.
    let rows = sqlx::query_as::<_, RawProductRow>(
        r#"
        SELECT
            p.id,
            p.organization_id,
            p.purchased_by,
            p.order_id,
            p.stripe_product_id,
            p.status,
            p.trial_ends_at,
            p.created_at,
            p.updated_at,
            CASE WHEN o.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', o.id,
                'organizationId', o.organization_id,
                'purchasedBy', o.purchased_by,
                'stripeCheckoutSessionId', o.stripe_checkout_session_id,
                'stripeCustomerId', o.stripe_customer_id,
                'stripeSubscriptionId', o.stripe_subscription_id,
                'stripeProductId', o.stripe_product_id,
                'stripePriceId', o.stripe_price_id,
                'subscriptionPrice', o.subscription_price,
                'subscriptionTerm', o.subscription_term,
                'status', o.status,
                'createdAt', o.created_at,
                'updatedAt', o.updated_at
            ) END AS "order",
            CASE WHEN sp.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', sp.id,
                'stripeProductId', sp.stripe_product_id,
                'name', sp.name,
                'description', sp.description,
                'active', sp.active,
                'statementDescriptor', sp.statement_descriptor,
                'unitLabel', sp.unit_label,
                'metadata', sp.metadata,
                'stripePrices', sp.stripe_prices,
                'createdAt', sp.created_at,
                'updatedAt', sp.updated_at
            ) END AS stripe_product
        FROM products p
        LEFT JOIN orders o ON o.id = p.order_id
        LEFT JOIN stripe_products sp ON sp.id = p.stripe_product_id
        WHERE p.organization_id = $1
        "#,
    )
    .bind(org_id)
    .fetch_all(&state.pool)
    .await?;

    let products: Vec<ProductWithRelations> = rows.into_iter().map(Into::into).collect();

    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(json) = serde_json::to_string(&products) {
            let _ = cache::set_string(&mut conn, &cache_key, &json, cache::SIDEBAR_TTL_SECS).await;
        }
    }

    Ok(Json(products))
}
