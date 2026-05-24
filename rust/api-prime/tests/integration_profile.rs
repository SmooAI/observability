//! Integration test for /v1/profile — verifies that requests with no
//! Authorization header are rejected at the handler layer (401) without
//! requiring a live Postgres or Redis.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use smooai_api_prime::test_support::build_router;
use smooai_api_prime::{auth::jwt::JwksCache, state::AppState};
use tower::ServiceExt;

fn lazy_state() -> AppState {
    // Lazy pool — no live connection required until first query.
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
async fn profile_requires_auth_header() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/profile")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn profile_rejects_non_bearer() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/profile")
                .header("Authorization", "Basic foo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn liveness_returns_ok() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(Request::builder().uri("/health/liveness").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn sign_in_rejects_invalid_email() {
    let app = build_router(lazy_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/sign-in")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"email":"not-an-email","password":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
