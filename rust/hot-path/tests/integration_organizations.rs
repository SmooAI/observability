//! Integration tests for the SMOODEV-1238 sidebar reads:
//! - `GET /v1/organizations`
//! - `GET /v1/organizations/:org_id/features`
//! - `GET /v1/organizations/:org_id/products`
//!
//! These mirror Rhea's `integration_profile.rs` approach: use a lazy
//! Postgres pool that never actually connects so we can exercise the
//! routing + auth-header parsing path without a live DB. Tests that
//! depend on a live database are marked `#[ignore]` so `cargo test`
//! still passes in CI but a developer can run them against a local
//! Supabase + seeded fixture via `cargo test -- --ignored`.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use smooai_hot_path::test_support::build_router;
use smooai_hot_path::{auth::jwt::JwksCache, state::AppState};
use tower::ServiceExt;

fn lazy_state() -> AppState {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
        .expect("lazy pool builds");
    let redis = redis::Client::open("redis://127.0.0.1:6379").expect("redis client builds");
    let jwks = JwksCache::new("https://example.invalid/auth/v1/.well-known/jwks.json".to_string());
    let http = reqwest::Client::new();
    AppState {
        pool,
        redis,
        jwks,
        http,
        supabase_url: "https://example.invalid".to_string(),
        supabase_anon_key: "anon".to_string(),
    }
}

#[tokio::test]
async fn list_organizations_requires_auth_header() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(Request::builder().uri("/v1/organizations").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_organizations_rejects_invalid_jwt() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations")
                .header("Authorization", "Bearer not-a-real-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Decoded header fails → AppError::Unauthorized.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn organization_features_requires_auth_header() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations/550e8400-e29b-41d4-a716-446655440000/features")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn organization_features_rejects_invalid_jwt() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations/550e8400-e29b-41d4-a716-446655440000/features")
                .header("Authorization", "Bearer not-a-real-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn organization_features_rejects_malformed_org_id() {
    let app = build_router(lazy_state());
    // axum returns 400 for a Path<Uuid> extractor failure — that comes
    // back before our auth check even runs. Either 400 or 401 is
    // defensible here; assert it's NOT a success so we don't fall through
    // to leaking anything.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations/not-a-uuid/features")
                .header("Authorization", "Bearer x")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn organization_products_requires_auth_header() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations/550e8400-e29b-41d4-a716-446655440000/products")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn organization_products_rejects_invalid_jwt() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations/550e8400-e29b-41d4-a716-446655440000/products")
                .header("Authorization", "Bearer not-a-real-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn organization_products_rejects_non_bearer() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations/550e8400-e29b-41d4-a716-446655440000/products")
                .header("Authorization", "Basic deadbeef")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Smoke test for the route table: a path with a trailing segment that
/// doesn't match any route returns 404, not 401. (This is what tells us
/// the router is wired up correctly — without it, you can't tell a
/// missing route from a missing auth header.)
#[tokio::test]
async fn unknown_route_returns_404() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/organizations/550e8400-e29b-41d4-a716-446655440000/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---- Live-DB tests (require a seeded local Supabase) ---------------------
//
// Run with `cargo test -- --ignored`. They need:
//   - DATABASE_URL pointing at a Postgres with our schema + at least one
//     organization, organization_member, product, and stripe_product row.
//   - A valid Supabase JWT in the SMOOAI_TEST_JWT env var.
// Skipped in CI per the same pattern Rhea's profile integration test
// uses for live-DB checks.

#[ignore]
#[tokio::test]
async fn features_forbidden_for_non_member_org() {
    // This test would set up a JWT for user A, then request features for
    // an org user A is NOT a member of, and assert 403. Skeleton only —
    // the test harness needs a seeded DB and a signed JWT to flesh out.
    // Filed under the same Phase 6 shadow-harness work as the other
    // end-to-end checks; integration_profile.rs has the same gap.
}
