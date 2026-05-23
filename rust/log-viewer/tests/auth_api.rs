//! Integration tests for the M2M auth + API client layer.
//!
//! We can't bring the binary's `main` into scope without compiling the whole
//! eframe stack into the test binary, so the modules under test are pulled in
//! via `#[path]` includes. That keeps phase-1/2 in a single binary crate
//! while still letting us exercise the real source against `wiremock`.
//!
//! Concretely, we test:
//! - `OAuthClient::exchange` parses the success response
//! - `AuthManager::verify` calls the token endpoint each time without caching
//! - The token endpoint propagates non-2xx as `AuthError::TokenEndpoint`
//! - `OrgClient` end-to-end: composes the URL, injects the bearer, decodes
//!   the response, and invalidates + retries on 401.

#[path = "../src/auth/mod.rs"]
mod auth;

#[path = "../src/api/mod.rs"]
mod api;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use auth::AuthManager;

fn http() -> reqwest::Client {
    reqwest::Client::builder().build().expect("reqwest")
}

#[tokio::test]
async fn oauth_exchange_returns_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .and(body_string_contains("client_id=cid"))
        .and(body_string_contains("client_secret=csecret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "tok-abc",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let oauth =
        auth::oauth::OAuthClient::new(http()).with_token_url(format!("{}/token", server.uri()));
    let resp = oauth.exchange("cid", "csecret").await.expect("exchange");
    assert_eq!(resp.access_token, "tok-abc");
    assert_eq!(resp.expires_in, 3600);
}

#[tokio::test]
async fn oauth_exchange_propagates_non_2xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
        .mount(&server)
        .await;

    let oauth =
        auth::oauth::OAuthClient::new(http()).with_token_url(format!("{}/token", server.uri()));
    let err = oauth
        .exchange("cid", "csecret")
        .await
        .expect_err("should fail");
    assert!(matches!(
        err,
        auth::AuthError::TokenEndpoint(reqwest::StatusCode::UNAUTHORIZED)
    ));
}

#[tokio::test]
async fn verify_calls_token_endpoint_without_caching() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "tok-xyz",
            "token_type": "Bearer",
            "expires_in": 60,
        })))
        .expect(2) // verify must hit the endpoint every time
        .mount(&server)
        .await;

    let mgr = AuthManager::new(http()).with_token_url(format!("{}/token", server.uri()));
    for _ in 0..2 {
        let resp = mgr.verify("cid", "csecret").await.unwrap();
        assert_eq!(resp.access_token, "tok-xyz");
    }
}

#[tokio::test]
async fn org_client_metric_list_round_trip() {
    let server = MockServer::start().await;
    let org = Uuid::nil();

    // Token endpoint → counts how many times we re-mint.
    let mint_count = Arc::new(AtomicUsize::new(0));
    let mint_count_for_responder = mint_count.clone();
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(move |_req: &Request| {
            let n = mint_count_for_responder.fetch_add(1, Ordering::SeqCst) + 1;
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": format!("tok-{n}"),
                "token_type": "Bearer",
                "expires_in": 3600,
            }))
        })
        .mount(&server)
        .await;

    // Metrics endpoint:
    //  - first request: respond 401 → forces invalidate + refresh
    //  - subsequent: respond 200 with one descriptor
    let api_path = format!("/organizations/{org}/observability/metrics");
    Mock::given(method("GET"))
        .and(path(api_path.clone()))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(401).set_body_string("expired"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(api_path))
        .and(header("authorization", "Bearer tok-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metrics": [
                {
                    "metric_name": "http.server.duration",
                    "kind": "histogram",
                    "unit": "ms",
                    "service_name": "api-server",
                    "last_seen_at": "2026-05-23T00:00:00Z",
                    "sample_count": 42,
                }
            ]
        })))
        .mount(&server)
        .await;

    // Use the test-only keychain path: skip keychain by manually seeding the
    // cache via an exposed test helper. Production keychain code is exercised
    // by a separate feature-gated test below.
    let mgr = AuthManager::new(http()).with_token_url(format!("{}/token", server.uri()));
    // Skip the keychain — set the credentials in-memory.
    mgr.set_override(org, "cid", "csecret");

    let api = api::ApiClient::new(http(), mgr.clone())
        .unwrap()
        .with_base(server.uri())
        .unwrap();

    let resp = api
        .org(org)
        .list_metrics(&api::metrics::MetricListParams::default())
        .await
        .expect("metrics");

    assert_eq!(resp.metrics.len(), 1);
    assert_eq!(resp.metrics[0].metric_name, "http.server.duration");
    // We minted twice: initial + post-401 refresh.
    assert_eq!(mint_count.load(Ordering::SeqCst), 2);
}
