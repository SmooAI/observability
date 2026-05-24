//! GET /v1/profile — port of `packages/backend/src/routes/profile.ts`.
//!
//! Differences from the TS route, documented here so the shadow harness
//! (Phase 6) can flag them if they matter:
//! - We rely entirely on the decoded JWT claims for the user id rather
//!   than calling Supabase `auth.getUser()`. Org context is intentionally
//!   not returned by this endpoint.
//! - `adminRoles` is queried via raw SQL (admin role IDs + names + timestamps)
//!   instead of going through Drizzle's relational mapper.
//! - Redis-backed read-through cache (`profile:{user_id}`, TTL 5min) is
//!   primed here. The TS route doesn't have this yet (planned in Phase 5b).

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

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AdminRole {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProfileRow {
    pub id: Uuid,
    pub email: String,
    pub phone: Option<String>,
    #[serde(rename = "fullName")]
    pub full_name: Option<String>,
    #[serde(rename = "smsOptIn")]
    pub sms_opt_in: bool,
    #[serde(rename = "smsOptInDate")]
    pub sms_opt_in_date: Option<DateTime<Utc>>,
    #[serde(rename = "emailOptIn")]
    pub email_opt_in: bool,
    #[serde(rename = "emailOptInDate")]
    pub email_opt_in_date: Option<DateTime<Utc>>,
    #[serde(rename = "marketingSmsOptIn")]
    pub marketing_sms_opt_in: bool,
    #[serde(rename = "marketingSmsOptInDate")]
    pub marketing_sms_opt_in_date: Option<DateTime<Utc>>,
    #[serde(rename = "marketingEmailOptIn")]
    pub marketing_email_opt_in: bool,
    #[serde(rename = "marketingEmailOptInDate")]
    pub marketing_email_opt_in_date: Option<DateTime<Utc>>,
    #[serde(rename = "profilePicture")]
    pub profile_picture: Option<String>,
    pub source: Option<Value>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileWithAdminRoles {
    #[serde(flatten)]
    pub profile: ProfileRow,
    #[serde(rename = "adminRoles")]
    pub admin_roles: Vec<AdminRole>,
}

pub async fn get_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProfileWithAdminRoles>, AppError> {
    let auth_header = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let token = extract_bearer(auth_header)?;

    let claims = state.jwks.verify(token).await?;
    let user_id = Uuid::from_str(&claims.sub).map_err(|_| AppError::Unauthorized("invalid user id in JWT".to_string()))?;

    // Read-through cache (best-effort — failures are non-fatal).
    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(Some(cached)) = cache::get_string(&mut conn, &cache::profile_key(&user_id.to_string())).await {
            if let Ok(parsed) = serde_json::from_str::<ProfileWithAdminRoles>(&cached) {
                return Ok(Json(parsed));
            }
        }
    }

    let profile = sqlx::query_as::<_, ProfileRow>(
        r#"
        SELECT
            id, email, phone, full_name, sms_opt_in, sms_opt_in_date,
            email_opt_in, email_opt_in_date, marketing_sms_opt_in,
            marketing_sms_opt_in_date, marketing_email_opt_in,
            marketing_email_opt_in_date, profile_picture, source,
            created_at, updated_at
        FROM profiles
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Profile not found".to_string()))?;

    let admin_roles = sqlx::query_as::<_, AdminRole>(
        r#"
        SELECT ar.id, ar.name::text AS name, ar.description, ar.created_at, ar.updated_at
        FROM admin_user_roles aur
        INNER JOIN admin_roles ar ON aur.role_id = ar.id
        WHERE aur.user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;

    let body = ProfileWithAdminRoles { profile, admin_roles };

    // Best-effort cache write.
    if let Ok(mut conn) = cache::get_connection(&state.redis).await {
        if let Ok(json) = serde_json::to_string(&body) {
            let _ = cache::set_string(&mut conn, &cache::profile_key(&user_id.to_string()), &json, cache::PROFILE_TTL_SECS).await;
        }
    }

    Ok(Json(body))
}
