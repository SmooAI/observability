//! GET /v1/organizations — port of `packages/backend/src/routes/organizations.ts`
//! (the GET list handler only — POST/PATCH stay on the Lambda side).
//!
//! Auth: Supabase JWT only (M2M provider lives in the TS gateway and
//! doesn't go through this service yet). Org-scoping is enforced in SQL
//! against `user_id = $1` from the JWT `sub` — the same trade Rhea
//! documented for /v1/profile (admin Postgres URL, no RLS, scoping
//! enforced by the query).
//!
//! Response shape parity: the TS handler returns rows from
//! `tx.query.organizations.findMany()` with an `accessType: 'member' | 'managed'`
//! tag. To stay close to the TS shape, we union direct memberships and
//! parent-admin-managed child orgs in a single SQL query and tag each
//! row.

use std::str::FromStr;

use axum::{
    extract::State,
    http::header::{HeaderMap, AUTHORIZATION},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::jwt::extract_bearer;
use crate::cache;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct OrganizationRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "primaryColor")]
    pub primary_color: Option<String>,
    #[serde(rename = "accentColor")]
    pub accent_color: Option<String>,
    pub logo: Option<String>,
    pub icon: Option<String>,
    #[serde(rename = "secondaryColor")]
    pub secondary_color: Option<String>,
    #[serde(rename = "harmonyMode")]
    pub harmony_mode: Option<String>,
    #[serde(rename = "logoWordmark")]
    pub logo_wordmark: Option<String>,
    #[serde(rename = "tailwindPalette")]
    pub tailwind_palette: Option<Value>,
    #[serde(rename = "metaData")]
    pub meta_data: Value,
    #[serde(rename = "createdBy")]
    pub created_by: Uuid,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
    /// `member` for direct memberships + created-by orgs, `managed` for
    /// child orgs reached via parent admin. Matches the TS handler.
    #[serde(rename = "accessType")]
    pub access_type: String,
}

pub async fn list_organizations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<OrganizationRow>>, AppError> {
    let auth_header = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let token = extract_bearer(auth_header)?;

    let claims = state.jwks.verify(token).await?;
    let user_id =
        Uuid::from_str(&claims.sub).map_err(|_| AppError::Unauthorized("invalid user id in JWT".to_string()))?;

    let cache_key = cache::organizations_key(&user_id.to_string());

    // Read-through cache — best-effort.
    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(Some(cached)) = cache::get_string(&mut conn, &cache_key).await {
            if let Ok(parsed) = serde_json::from_str::<Vec<OrganizationRow>>(&cached) {
                return Ok(Json(parsed));
            }
        }
    }

    // Direct memberships + creator-orgs. We DISTINCT-merge the two cohorts
    // and tag every row `member`. A separate query then pulls
    // parent-admin-managed child orgs and tags them `managed`. Order of
    // operations matters: a row that lands in BOTH cohorts must be tagged
    // `member`, mirroring the TS handler's
    // `managedOrgIds.has(org.id) ? 'managed' : 'member'` check.
    let member_orgs = sqlx::query_as::<_, OrganizationRow>(
        r#"
        SELECT DISTINCT
            o.id,
            o.name,
            o.description,
            o.primary_color,
            o.accent_color,
            o.logo,
            o.icon,
            o.secondary_color,
            o.harmony_mode,
            o.logo_wordmark,
            o.tailwind_palette,
            o.meta_data,
            o.created_by,
            o.created_at,
            o.updated_at,
            'member'::text AS access_type
        FROM organizations o
        WHERE o.id IN (
            SELECT om.organization_id
            FROM organization_members om
            WHERE om.user_id = $1
        )
        OR o.created_by = $1
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;

    let member_ids: std::collections::HashSet<Uuid> = member_orgs.iter().map(|o| o.id).collect();

    let managed_orgs = sqlx::query_as::<_, OrganizationRow>(
        r#"
        SELECT DISTINCT
            o.id,
            o.name,
            o.description,
            o.primary_color,
            o.accent_color,
            o.logo,
            o.icon,
            o.secondary_color,
            o.harmony_mode,
            o.logo_wordmark,
            o.tailwind_palette,
            o.meta_data,
            o.created_by,
            o.created_at,
            o.updated_at,
            'managed'::text AS access_type
        FROM organizations o
        INNER JOIN organization_relationships orr ON orr.child_org_id = o.id
        INNER JOIN organization_members om ON om.organization_id = orr.parent_org_id
        WHERE om.user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;

    // Merge: keep all `member` rows, then add `managed` rows whose ids
    // aren't already in the member set.
    let mut combined = member_orgs;
    for org in managed_orgs {
        if !member_ids.contains(&org.id) {
            combined.push(org);
        }
    }

    // Best-effort cache write — 60s TTL.
    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(json) = serde_json::to_string(&combined) {
            let _ = cache::set_string(&mut conn, &cache_key, &json, cache::SIDEBAR_TTL_SECS).await;
        }
    }

    Ok(Json(combined))
}
