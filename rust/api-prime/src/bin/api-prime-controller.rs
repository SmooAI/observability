//! api-prime-controller: control-plane binary for the api-prime edge mesh.
//!
//! See ADR-017 (Edge Mesh / api-prime split) for the full design. This is
//! the skeleton — Wave 3 fills in the actual logic. Currently exposes only:
//!
//!   GET  /admin/v1/health              — liveness + identifies the binary
//!   POST /internal/v1/cache/invalidate — stub, returns 204
//!
//! Architecture (single replica, owns route table state in Valkey):
//!   - Reconcile loop: watch the route manifest package + Lambda function
//!     registry, compute the desired route table, write into Valkey.
//!   - Admin API: human-facing, behind admin.smoo.ai/api-prime/*. Surfaces
//!     route table inspection, manual reconcile triggers, drain/cordon.
//!   - Internal API: cluster-only ClusterIP. Data-plane pods hit
//!     /internal/v1/cache/invalidate when their local cache needs to be
//!     refreshed (e.g., admin operator changes a route).
//!   - Lambda health probing: periodic best-effort pings to back-end
//!     Lambdas so the controller knows which targets are healthy before
//!     handing the route table to the data plane.
//!   - OpenAPI generation: emit the merged OpenAPI document from the
//!     route manifest so docs / SDK generators have a single source.
//!
//! Wave 3 work to fill in (tracked under SMOODEV-1272's child tickets):
//!   - reconcile loop wired to the manifest TS package output (S3 blob)
//!   - admin endpoints: GET /admin/v1/routes, POST /admin/v1/reconcile, …
//!   - internal endpoints: full cache invalidation protocol w/ ETag
//!   - lambda health probing + circuit breaker
//!   - OpenAPI emission from the merged route manifest

use std::net::SocketAddr;

use anyhow::Context;
use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    tracing::info!(port, "starting api-prime-controller (control plane skeleton)");

    let app = Router::new()
        .route("/admin/v1/health", get(admin_health))
        .route("/internal/v1/cache/invalidate", post(internal_cache_invalidate))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse().context("invalid bind address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {}", addr))?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

async fn admin_health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "binary": "api-prime-controller" }))
}

async fn internal_cache_invalidate() -> StatusCode {
    // Wave 3: read body { keys: [...] }, fan out to data-plane pods,
    // update Valkey ETag. For now we just acknowledge.
    StatusCode::NO_CONTENT
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry().with(filter).with(fmt::layer().json()).init();
}
