//! Integration tests for `/v1/auth/sign-in` + `/v1/auth/refresh`.
//!
//! Uses `wiremock` to stand up a fake Supabase GoTrue server so we can
//! exercise the full happy-path body passthrough + the error mappings
//! without a live Supabase project.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use smooai_hot_path::{auth::jwt::JwksCache, state::AppState, test_support::build_router};
use tower::ServiceExt;
use wiremock::{
    matchers::{body_json, header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

/// Build an AppState pointed at the given mock Supabase URL.
///
/// Each test passes its own Redis DB index by including the test name in
/// the rate-limit key — but Redis isn't actually required for these tests
/// to pass (the rate limiter fails open when Redis is unreachable). We use
/// an obviously-unreachable address so we don't accidentally clobber a real
/// Redis if one is running locally.
fn state_with_supabase(url: String) -> AppState {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
        .expect("lazy pool builds");
    // Unreachable redis — rate-limiter fails open, which is what we want
    // for the upstream-mapping tests.
    let redis = redis::Client::open("redis://127.0.0.1:1/").expect("redis client builds");
    let jwks = JwksCache::new(format!("{}/auth/v1/.well-known/jwks.json", url));
    let http = reqwest::Client::new();
    AppState {
        pool,
        redis,
        jwks,
        http,
        supabase_url: url,
        supabase_anon_key: "test-anon-key".to_string(),
    }
}

async fn read_json(response: axum::response::Response) -> Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap_or(Value::Null)
}

#[tokio::test]
async fn sign_in_success_returns_supabase_body() {
    let server = MockServer::start().await;
    let supabase_body = json!({
        "access_token": "eyJfake",
        "refresh_token": "ref_fake",
        "expires_in": 3600,
        "expires_at": 1_900_000_000,
        "token_type": "bearer",
        "user": {"id": "u1", "email": "a@b.co"}
    });
    Mock::given(method("POST"))
        .and(path("/auth/v1/token"))
        .and(query_param("grant_type", "password"))
        .and(header("apikey", "test-anon-key"))
        .and(body_json(json!({"email": "a@b.co", "password": "pw"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&supabase_body))
        .mount(&server)
        .await;

    let app = build_router(state_with_supabase(server.uri()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/sign-in")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"email":"a@b.co","password":"pw"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body, supabase_body);
}

#[tokio::test]
async fn sign_in_wrong_password_maps_to_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": "invalid_grant", "error_description": "Invalid login credentials"})),
        )
        .mount(&server)
        .await;

    let app = build_router(state_with_supabase(server.uri()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/sign-in")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"email":"a@b.co","password":"wrong"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = read_json(response).await;
    assert_eq!(body["message"], "Invalid login credentials");
    // Ensure we didn't leak GoTrue's error envelope.
    assert!(body.get("error").is_none());
    assert!(body.get("error_description").is_none());
}

#[tokio::test]
async fn sign_in_supabase_5xx_maps_to_502() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/token"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let app = build_router(state_with_supabase(server.uri()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/sign-in")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"email":"a@b.co","password":"pw"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = read_json(response).await;
    assert_eq!(body["message"], "Auth provider unavailable");
}

#[tokio::test]
async fn sign_in_rate_limit_returns_429() {
    // This test requires a reachable Redis to exercise the rate-limiter
    // path (when Redis is down the limiter fails open). The CI runner runs
    // a local Redis on 6379, so we use it here. If Redis isn't up, the
    // test is meaningless — assert success rather than 429.
    let redis_client = match redis::Client::open("redis://127.0.0.1:6379") {
        Ok(c) => c,
        Err(_) => return, // No Redis configured — silently skip.
    };
    let probe = redis_client.get_multiplexed_async_connection().await;
    if probe.is_err() {
        return; // Redis not reachable — skip.
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "x", "refresh_token": "y"})))
        .mount(&server)
        .await;

    let mut state = state_with_supabase(server.uri());
    state.redis = redis_client;

    // Clear any prior counter for this IP. Tests run on the same loopback,
    // so without a fresh key we'd inherit drift from earlier runs.
    let ip = "203.0.113.99";
    let key = format!("auth:signin:{}", ip);
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        let _: Result<i64, _> = redis::cmd("DEL").arg(&key).query_async(&mut conn).await;
    }

    let app = build_router(state);
    // Fire 11 requests through XFF — the 11th should be rate-limited.
    let mut last_status = StatusCode::OK;
    for _ in 0..11 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/sign-in")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", ip)
                    .body(Body::from(r#"{"email":"a@b.co","password":"pw"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        last_status = response.status();
    }
    assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn refresh_success_returns_supabase_body() {
    let server = MockServer::start().await;
    let supabase_body = json!({
        "access_token": "eyJnew",
        "refresh_token": "ref_new",
        "expires_in": 3600,
        "token_type": "bearer",
        "user": {"id": "u1", "email": "a@b.co"}
    });
    Mock::given(method("POST"))
        .and(path("/auth/v1/token"))
        .and(query_param("grant_type", "refresh_token"))
        .and(header("apikey", "test-anon-key"))
        .and(body_json(json!({"refresh_token": "ref_old"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&supabase_body))
        .mount(&server)
        .await;

    let app = build_router(state_with_supabase(server.uri()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"refresh_token":"ref_old"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body, supabase_body);
}

#[tokio::test]
async fn refresh_bad_token_maps_to_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v1/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .mount(&server)
        .await;

    let app = build_router(state_with_supabase(server.uri()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"refresh_token":"bad"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = read_json(response).await;
    assert_eq!(body["message"], "Invalid refresh token");
}
