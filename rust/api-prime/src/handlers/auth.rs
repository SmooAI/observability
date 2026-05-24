//! POST /v1/auth/sign-in and POST /v1/auth/refresh — Supabase GoTrue passthrough.
//!
//! Both endpoints proxy to the Supabase Auth REST API and return the GoTrue
//! response body verbatim on success. The dashboard's server action consumes
//! `access_token` + `refresh_token` and hands them to `@supabase/ssr`'s
//! `setSession()` to write cookies in the standard `sb-{ref}-auth-token`
//! format — so we don't try to write cookies ourselves.
//!
//! ## Why proxy instead of letting the dashboard call Supabase directly?
//!
//! 1. Skips the Node Lambda cold start (the whole point of this crate).
//! 2. Lets us rate-limit by IP in front of Supabase using the shared Redis,
//!    rather than relying on Supabase's per-project bucket.
//! 3. Lets us normalize Supabase's error envelope into our own
//!    `{message: "..."}` shape (don't leak GoTrue specifics to the client).
//!
//! ## Endpoints
//!
//! - `POST /v1/auth/sign-in`  body: `{email, password}`  rate-limited 10/60s per IP.
//! - `POST /v1/auth/refresh`  body: `{refresh_token}`    no extra rate limit (Supabase RLs refresh tokens).

use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts, State},
    http::{header::HeaderMap, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use redis::AsyncCommands;
use serde::Deserialize;
use serde_json::Value;
use std::convert::Infallible;
use std::net::SocketAddr;

/// Optional ConnectInfo — falls back to `None` instead of erroring when the
/// router wasn't built with `into_make_service_with_connect_info` (e.g. in
/// `oneshot`-based integration tests).
pub struct MaybePeer(Option<SocketAddr>);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for MaybePeer {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(MaybePeer(
            ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
                .await
                .ok()
                .map(|ConnectInfo(addr)| addr),
        ))
    }
}

use crate::error::AppError;
use crate::state::AppState;

// Rate-limit knobs. Per-IP, sliding 60s window via Redis INCR + EXPIRE.
const SIGN_IN_RATE_LIMIT: u32 = 10;
const SIGN_IN_RATE_WINDOW_SECS: u64 = 60;

#[derive(Debug, Deserialize)]
pub struct SignInRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Very loose email shape check — full validation belongs to Supabase.
/// We only reject obvious garbage so we don't waste a network roundtrip.
fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 3 || s.len() > 254 {
        return false;
    }
    let at = match s.find('@') {
        Some(i) => i,
        None => return false,
    };
    if at == 0 || at == s.len() - 1 {
        return false;
    }
    let (local, domain) = s.split_at(at);
    let domain = &domain[1..];
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// Extract the caller IP. Prefer `X-Forwarded-For` (left-most) since the
/// service sits behind an ingress; fall back to the socket peer, then to
/// `unknown` if neither is available (test contexts).
fn caller_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let trimmed = first.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    if let Some(real) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if !real.is_empty() {
            return real.to_string();
        }
    }
    peer.map(|p| p.ip().to_string()).unwrap_or_else(|| "unknown".to_string())
}

/// Increment the per-IP counter; return Err(retry_after_secs) when over the limit.
async fn check_rate_limit(state: &AppState, ip: &str) -> Result<(), u64> {
    let key = format!("auth:signin:{}", ip);
    let mut conn = match state.redis.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(err) => {
            // Fail-open on Redis outage — better to let auth through than 500.
            tracing::warn!(error = %err, "rate-limit redis unavailable; failing open");
            return Ok(());
        }
    };
    let count: i64 = match conn.incr(&key, 1).await {
        Ok(n) => n,
        Err(err) => {
            tracing::warn!(error = %err, "rate-limit INCR failed; failing open");
            return Ok(());
        }
    };
    if count == 1 {
        // First hit — set TTL.
        let _: Result<(), _> = conn.expire(&key, SIGN_IN_RATE_WINDOW_SECS as i64).await;
    }
    if count > SIGN_IN_RATE_LIMIT as i64 {
        let ttl: i64 = conn.ttl(&key).await.unwrap_or(SIGN_IN_RATE_WINDOW_SECS as i64);
        let retry = if ttl > 0 { ttl as u64 } else { SIGN_IN_RATE_WINDOW_SECS };
        return Err(retry);
    }
    Ok(())
}

pub async fn sign_in(
    State(state): State<AppState>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Json(req): Json<SignInRequest>,
) -> Result<Response, AppError> {
    if !looks_like_email(&req.email) {
        return Err(AppError::BadRequest("Invalid email".to_string()));
    }
    if req.password.is_empty() {
        return Err(AppError::BadRequest("Password required".to_string()));
    }

    let ip = caller_ip(&headers, peer);
    if let Err(retry) = check_rate_limit(&state, &ip).await {
        tracing::warn!(ip = %ip, "sign-in rate limited");
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"message": "Too many sign-in attempts. Try again shortly."})),
        )
            .into_response();
        resp.headers_mut().insert(
            "Retry-After",
            retry.to_string().parse().expect("u64 is valid header value"),
        );
        return Ok(resp);
    }

    let url = format!(
        "{}/auth/v1/token?grant_type=password",
        state.supabase_url.trim_end_matches('/')
    );

    let upstream = state
        .http
        .post(&url)
        .header("apikey", &state.supabase_anon_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"email": req.email, "password": req.password}))
        .send()
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "supabase sign-in request failed");
            AppError::Internal("Auth provider unavailable".to_string())
        })?;

    let status = upstream.status();
    let body_bytes = upstream.bytes().await.map_err(|err| {
        tracing::error!(error = %err, "supabase sign-in body read failed");
        AppError::Internal("Auth provider unavailable".to_string())
    })?;

    if status.is_success() {
        // Pass body verbatim — dashboard hands it to supabase.auth.setSession().
        let value: Value = serde_json::from_slice(&body_bytes)
            .map_err(|err| {
                tracing::error!(error = %err, "supabase sign-in body parse failed");
                AppError::Internal("Auth provider returned invalid response".to_string())
            })?;
        tracing::info!(email = %req.email, "sign-in success");
        return Ok((StatusCode::OK, Json(value)).into_response());
    }

    if status.as_u16() == 400 || status.as_u16() == 401 {
        // Don't surface GoTrue's specific error; map all 4xx-credential
        // failures to a single "invalid credentials" line.
        tracing::info!(email = %req.email, status = %status, "sign-in rejected");
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"message": "Invalid login credentials"})),
        )
            .into_response());
    }

    if status.as_u16() == 429 {
        tracing::warn!(email = %req.email, "supabase rate-limited sign-in");
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"message": "Too many sign-in attempts. Try again shortly."})),
        )
            .into_response());
    }

    // 5xx (or anything else) from Supabase → 502 to caller.
    tracing::error!(status = %status, body = %String::from_utf8_lossy(&body_bytes), "supabase sign-in upstream error");
    Ok((
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({"message": "Auth provider unavailable"})),
    )
        .into_response())
}

pub async fn refresh(State(state): State<AppState>, Json(req): Json<RefreshRequest>) -> Result<Response, AppError> {
    if req.refresh_token.is_empty() {
        return Err(AppError::BadRequest("refresh_token required".to_string()));
    }

    let url = format!(
        "{}/auth/v1/token?grant_type=refresh_token",
        state.supabase_url.trim_end_matches('/')
    );

    let upstream = state
        .http
        .post(&url)
        .header("apikey", &state.supabase_anon_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"refresh_token": req.refresh_token}))
        .send()
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "supabase refresh request failed");
            AppError::Internal("Auth provider unavailable".to_string())
        })?;

    let status = upstream.status();
    let body_bytes = upstream.bytes().await.map_err(|err| {
        tracing::error!(error = %err, "supabase refresh body read failed");
        AppError::Internal("Auth provider unavailable".to_string())
    })?;

    if status.is_success() {
        let value: Value = serde_json::from_slice(&body_bytes)
            .map_err(|_| AppError::Internal("Auth provider returned invalid response".to_string()))?;
        return Ok((StatusCode::OK, Json(value)).into_response());
    }

    if status.as_u16() == 400 || status.as_u16() == 401 {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"message": "Invalid refresh token"})),
        )
            .into_response());
    }

    tracing::error!(status = %status, body = %String::from_utf8_lossy(&body_bytes), "supabase refresh upstream error");
    Ok((
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({"message": "Auth provider unavailable"})),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation_accepts_normal_addresses() {
        assert!(looks_like_email("a@b.co"));
        assert!(looks_like_email("user.name+tag@example.co.uk"));
    }

    #[test]
    fn email_validation_rejects_garbage() {
        assert!(!looks_like_email(""));
        assert!(!looks_like_email("no-at-sign"));
        assert!(!looks_like_email("@nolocal.com"));
        assert!(!looks_like_email("nodomain@"));
        assert!(!looks_like_email("nodot@invalid"));
        assert!(!looks_like_email("trailing.dot@example."));
    }
}
