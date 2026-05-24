//! Admin API for the api-prime controller. Routes are all under
//! `/admin/v1/*` and proxied through the admin Ingress at
//! `admin.smoo.ai/api-prime/...`.
//!
//! Auth is intentionally a TODO right now — Phase 2 wires real Supabase
//! JWT verification + admin-org-membership check. Phase 1 trusts the
//! presence of `X-Smoo-Admin-Authenticated: true`, which the Ingress sets
//! after the dashboard's admin auth subrequest succeeds.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::controller::internal::{self, perform_invalidation, ControllerState, InvalidateRequest};
use crate::controller::pubsub;
use crate::controller::types::{ResolvedRouteEntry, RouteMode};

pub const HEADER_ADMIN_AUTH: &str = "x-smoo-admin-authenticated";

pub fn router(state: ControllerState) -> Router {
    Router::new()
        .route("/admin/v1/health", get(health))
        .route("/admin/v1/routes", get(list_routes))
        .route("/admin/v1/routes/:id", get(get_route))
        .route("/admin/v1/routes/:id/mode", put(update_route_mode))
        .route(
            "/admin/v1/routes/:id/cache/invalidate",
            post(invalidate_route_cache),
        )
        .route("/admin/v1/openapi.json", get(openapi_stub))
        // Internal endpoint registered here too so we share a single
        // `ControllerState` axum router rather than juggling two.
        .route("/internal/v1/cache/invalidate", post(internal::invalidate))
        .with_state(state)
}

/// Phase 1 admin auth gate. The Ingress is expected to terminate the JWT
/// check via subrequest and append this header on success. Phase 2 (see
/// SMOODEV-1283 follow-up) will verify a Supabase admin JWT here.
///
/// Returns `Some(Response)` to short-circuit handling with the rejection
/// response, or `None` when auth passed. We avoid `Result<(), Response>`
/// here because clippy flags the 128-byte `Response` Err variant.
fn require_admin(headers: &HeaderMap, dev_bypass: bool) -> Option<Response> {
    if dev_bypass {
        return None;
    }
    match headers.get(HEADER_ADMIN_AUTH).and_then(|v| v.to_str().ok()) {
        Some("true") => None,
        _ => Some((StatusCode::UNAUTHORIZED, "admin auth required").into_response()),
    }
}

pub async fn health(State(state): State<ControllerState>) -> Json<serde_json::Value> {
    let snap = state.snapshot.read().await;
    let uptime = (chrono::Utc::now() - state.started_at).num_seconds().max(0);
    Json(json!({
        "ok": true,
        "binary": "api-prime-controller",
        "uptime_seconds": uptime,
        "reconcile_last_at": snap.last_reconcile_at,
        "reconcile_last_status": snap.last_reconcile_status.as_str(),
        "routes_count": snap.routes.len(),
    }))
}

/// `GET /admin/v1/routes`. Reads from the in-memory reconcile snapshot —
/// the admin API never touches Valkey for reads; the snapshot is the
/// canonical view from the controller's perspective.
pub async fn list_routes(State(state): State<ControllerState>, headers: HeaderMap) -> Response {
    if let Some(resp) = require_admin(&headers, state.admin_dev_bypass) {
        return resp;
    }
    let snap = state.snapshot.read().await;
    Json(&snap.routes).into_response()
}

/// `GET /admin/v1/routes/:id`. The `id` path param is URL-encoded
/// `METHOD:path` (e.g., `GET:%2Fv1%2Fprofile`).
pub async fn get_route(
    State(state): State<ControllerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(resp) = require_admin(&headers, state.admin_dev_bypass) {
        return resp;
    }
    let decoded = match urlencoding::decode(&id) {
        Ok(s) => s.into_owned(),
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid id encoding").into_response(),
    };
    let snap = state.snapshot.read().await;
    match snap.routes.iter().find(|r| r.route_id() == decoded) {
        Some(r) => Json(r).into_response(),
        None => (StatusCode::NOT_FOUND, "route not found").into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateModeRequest {
    pub mode: RouteMode,
}

#[derive(Debug, Serialize)]
struct UpdateModeError {
    error: String,
}

/// `PUT /admin/v1/routes/:id/mode`. Updates the route's mode in Valkey,
/// publishes `apr:config-bump`, and patches the in-memory snapshot so
/// subsequent reads reflect the change immediately.
///
/// Validation: `implement` mode requires a registered Rust handler. The
/// existing `implement.rustHandler` from the manifest is checked against
/// the controller's registered-handlers list.
pub async fn update_route_mode(
    State(state): State<ControllerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateModeRequest>,
) -> Response {
    if let Some(resp) = require_admin(&headers, state.admin_dev_bypass) {
        return resp;
    }
    let decoded = match urlencoding::decode(&id) {
        Ok(s) => s.into_owned(),
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid id encoding").into_response(),
    };

    // Find + clone the route so we can release the read lock before doing
    // I/O. We re-acquire a write lock at the end to commit the change.
    let mut updated: ResolvedRouteEntry = {
        let snap = state.snapshot.read().await;
        match snap.routes.iter().find(|r| r.route_id() == decoded) {
            Some(r) => r.clone(),
            None => return (StatusCode::NOT_FOUND, "route not found").into_response(),
        }
    };

    // Validate implement-mode handler is registered.
    if matches!(req.mode, RouteMode::Implement) {
        let handler = updated
            .entry
            .implement
            .as_ref()
            .map(|i| i.rust_handler.as_str());
        match handler {
            Some(name) if crate::controller::is_registered_handler(name) => {}
            Some(name) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(UpdateModeError {
                        error: format!(
                            "implement.rustHandler '{}' is not registered in this build",
                            name
                        ),
                    }),
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(UpdateModeError {
                        error: "route has no implement.rustHandler — manifest must declare one before mode=implement".into(),
                    }),
                )
                    .into_response();
            }
        }
    }

    updated.entry.mode = req.mode.clone();

    // Persist to Valkey + bump config.
    let payload = match serde_json::to_string(&updated) {
        Ok(s) => s,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize failed: {}", err),
            )
                .into_response()
        }
    };
    let key = updated.valkey_key();

    let mut conn = match state.redis.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(error = %err, "redis unreachable in update_route_mode");
            return (StatusCode::SERVICE_UNAVAILABLE, "valkey unreachable").into_response();
        }
    };
    if let Err(err) = conn.set::<_, _, ()>(&key, &payload).await {
        tracing::error!(error = %err, key = %key, "SET failed in update_route_mode");
        return (StatusCode::SERVICE_UNAVAILABLE, "valkey write failed").into_response();
    }
    if let Err(err) = pubsub::publish_config_bump(&mut conn).await {
        tracing::error!(error = %err, "publish_config_bump failed in update_route_mode");
        // The write succeeded; data plane will eventually pick this up on
        // the next reconcile + bump. Don't fail the request.
    }

    // Update in-memory snapshot so the very next GET reflects the change.
    {
        let mut snap = state.snapshot.write().await;
        if let Some(slot) = snap.routes.iter_mut().find(|r| r.route_id() == decoded) {
            *slot = updated.clone();
        }
    }

    Json(updated).into_response()
}

/// `POST /admin/v1/routes/:id/cache/invalidate`. The route id is currently
/// unused in the invalidation logic (we invalidate by tags, not by route)
/// but is kept in the URL so it's clear from access logs which route was
/// being operated on.
pub async fn invalidate_route_cache(
    State(state): State<ControllerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<InvalidateRequest>,
) -> Response {
    if let Some(resp) = require_admin(&headers, state.admin_dev_bypass) {
        return resp;
    }
    if req.tags.is_empty() {
        return (StatusCode::BAD_REQUEST, "tags must be non-empty").into_response();
    }
    tracing::info!(route_id = %id, tag_count = req.tags.len(), "admin cache invalidate");
    match perform_invalidation(&state, &req.tags).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "admin invalidate failed");
            (StatusCode::SERVICE_UNAVAILABLE, "valkey unreachable").into_response()
        }
    }
}

/// `GET /admin/v1/openapi.json`. Stub — Wave 4 wires this through the
/// manifest's generator output. Returns an empty paths object for now so
/// the dashboard's OpenAPI viewer doesn't 404.
pub async fn openapi_stub() -> Json<serde_json::Value> {
    // TODO(SMOODEV-1283 follow-up): emit the merged OpenAPI document from
    // the route manifest at `/etc/api-prime/openapi.json` (also mounted
    // via ConfigMap alongside the route manifest).
    Json(json!({
        "openapi": "3.1.0",
        "info": { "title": "api-prime", "version": "0.1.0" },
        "paths": {},
    }))
}

// Re-exported so the bin can use the same `Arc` type for the snapshot lock.
pub type SnapshotLock = Arc<tokio::sync::RwLock<crate::controller::reconcile::ReconcileSnapshot>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::reconcile::{ReconcileSnapshot, ReconcileStatus};
    use crate::controller::types::{
        AuthClass, ImplementConfig, RateLimitConfig, ResolvedRouteEntry, RouteEntry, RouteMode,
    };
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    fn fixture_state() -> ControllerState {
        let snapshot = Arc::new(tokio::sync::RwLock::new(ReconcileSnapshot {
            routes: vec![ResolvedRouteEntry {
                entry: RouteEntry {
                    path: "/v1/profile".to_string(),
                    method: "GET".to_string(),
                    auth: AuthClass::User,
                    idempotent: true,
                    mode: RouteMode::Proxy,
                    rate_limit: RateLimitConfig {
                        per_token: 100,
                        window_seconds: 60,
                    },
                    cache: None,
                    implement: Some(ImplementConfig {
                        rust_handler: "profile".to_string(),
                    }),
                    lambda_output_key: Some("ApiRouteProfile".to_string()),
                    schema_ref: "Profile".to_string(),
                },
                lambda_arn: Some("arn:aws:lambda:us-east-1:1:function:Profile".to_string()),
            }],
            last_reconcile_at: Some(chrono::Utc::now()),
            last_reconcile_status: ReconcileStatus::Ok,
        }));
        ControllerState {
            // Redis won't be touched in admin tests that don't write — but
            // we still need a client. localhost:6379 is only contacted if
            // the test exercises a write path; the tests below stick to
            // reads.
            redis: redis::Client::open("redis://127.0.0.1:6379").unwrap(),
            edge_attest_secret: Arc::new("test-secret".to_string()),
            snapshot,
            started_at: chrono::Utc::now(),
            admin_dev_bypass: true,
        }
    }

    #[tokio::test]
    async fn list_routes_returns_snapshot() {
        let state = fixture_state();
        let app = router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/v1/routes")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["path"], "/v1/profile");
    }

    #[tokio::test]
    async fn get_route_returns_404_when_missing() {
        let state = fixture_state();
        let app = router(state);
        let id = urlencoding::encode("POST:/nope");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/admin/v1/routes/{}", id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn health_reports_routes_count_and_status() {
        let state = fixture_state();
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/v1/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["binary"], "api-prime-controller");
        assert_eq!(v["routes_count"], 1);
        assert_eq!(v["reconcile_last_status"], "ok");
    }

    #[tokio::test]
    async fn openapi_stub_is_well_formed() {
        let state = fixture_state();
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/v1/openapi.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["openapi"], "3.1.0");
    }

    #[tokio::test]
    async fn admin_auth_required_when_dev_bypass_off() {
        let mut state = fixture_state();
        state.admin_dev_bypass = false;
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/v1/routes")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
