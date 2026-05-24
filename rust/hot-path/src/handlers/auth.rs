//! POST /v1/auth/sign-in — STUB.
//!
//! This handler is intentionally a scaffold only. Phase 5d (next agent)
//! fills in the real password sign-in flow. Below is the contract + a
//! checklist of everything the implementer needs.
//!
//! # What's missing
//!
//! 1. Call Supabase Auth REST:
//!
//! ```text
//! POST {SUPABASE_URL}/auth/v1/token?grant_type=password
//! Headers:
//!   apikey: {SUPABASE_ANON_KEY}
//!   Content-Type: application/json
//! Body: { "email": "...", "password": "..." }
//! ```
//!
//! Use `state.http` (already a configured reqwest client) and
//! `state.supabase_url` + `state.supabase_anon_key` from `AppState`.
//!
//! 2. Map Supabase error responses to AppError:
//!
//! ```text
//! 400 invalid_grant            -> AppError::Unauthorized("Invalid credentials")
//! 400 email_not_confirmed      -> AppError::BadRequest(...)
//! 429 over_email_send_rate_limit -> AppError::BadRequest(...)
//! other                        -> AppError::Internal
//! ```
//!
//! 3. Cookie shape. The TS dashboard uses Supabase JS which writes cookies
//!    of the form:
//!
//! ```text
//! sb-{project-ref}-auth-token=base64-eyJhY2Nlc3NfdG9rZW4i...
//! ```
//!
//!    with `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`, and
//!    `Max-Age=session_expires`. For an API-driven flow we should return
//!    the `access_token` + `refresh_token` in the JSON body and let the
//!    dashboard's auth helper write the cookies — matches how the existing
//!    TS sign-in route works.
//!
//! 4. Response shape (proposed — confirm with Wave 2 agent against
//!    whatever the dashboard expects):
//!
//! ```json
//! {
//!   "access_token": "...",
//!   "refresh_token": "...",
//!   "expires_in": 3600,
//!   "expires_at": 1730000000,
//!   "token_type": "bearer",
//!   "user": { "id": "...", "email": "..." }
//! }
//! ```
//!
//! 5. Rate limiting. Sign-in should be rate-limited by IP (e.g. 10/min) —
//!    add a tower middleware before this route. Redis-backed counter
//!    via the existing `state.redis` is fine.
//!
//! 6. Audit log + structured tracing on every attempt (success + failure).

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SignInRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SignInResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub expires_at: i64,
    pub token_type: String,
    pub user: SignedInUser,
}

#[derive(Debug, Serialize)]
pub struct SignedInUser {
    pub id: String,
    pub email: String,
}

pub async fn sign_in(
    State(_state): State<AppState>,
    Json(_req): Json<SignInRequest>,
) -> Result<Json<SignInResponse>, AppError> {
    // TODO(SMOODEV-1227-wave2): implement per docstring above.
    Err(AppError::NotImplemented(
        "POST /v1/auth/sign-in is a scaffold — Phase 5d wires up Supabase password grant".to_string(),
    ))
}
