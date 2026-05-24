//! api-prime: axum HTTP data-plane service serving high-traffic read endpoints.
//!
//! Renamed from `smooai-hot-path` per ADR-017 (Edge Mesh / api-prime split).
//! This is the data-plane binary; the control-plane lives in
//! `src/bin/api-prime-controller.rs`. Wave 3 fills in routing + edge logic.
//!
//! Endpoints exposed today (unchanged behavior from the hot-path crate):
//! - `GET  /health/liveness`             — process alive
//! - `GET  /health/readiness`            — pool + redis reachable
//! - `GET  /v1/profile`                  — Supabase-JWT-authenticated profile read
//! - `POST /v1/auth/sign-in`             — Supabase password grant passthrough (rate-limited per IP)
//! - `POST /v1/auth/refresh`             — Supabase refresh-token grant passthrough

use std::net::SocketAddr;

use anyhow::Context;
use axum::{
    routing::{get, post},
    Router,
};
use smooai_api_prime::{auth::jwt::JwksCache, cache, db, handlers, state::AppState};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!(port = config.port, "starting api-prime (data plane)");

    let pool = db::init_pool(&config.database_url)
        .await
        .context("failed to initialize Postgres pool")?;

    let redis_client = cache::init_client(&config.redis_url).context("failed to initialize Redis client")?;

    let jwks = JwksCache::new(config.supabase_jwks_url.clone());

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build reqwest client")?;

    let state = AppState {
        pool,
        redis: redis_client,
        jwks,
        http,
        supabase_url: config.supabase_url,
        supabase_anon_key: config.supabase_anon_key,
    };

    let app = Router::new()
        .route("/health/liveness", get(handlers::health::liveness))
        .route("/health/readiness", get(handlers::health::readiness))
        .route("/v1/profile", get(handlers::profile::get_profile))
        .route("/v1/auth/sign-in", post(handlers::auth::sign_in))
        .route("/v1/auth/refresh", post(handlers::auth::refresh))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry().with(filter).with(fmt::layer().json()).init();
}

struct Config {
    database_url: String,
    redis_url: String,
    supabase_url: String,
    supabase_anon_key: String,
    supabase_jwks_url: String,
    port: u16,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let supabase_url = std::env::var("SUPABASE_URL").context("SUPABASE_URL is required")?;
        let supabase_anon_key = std::env::var("SUPABASE_ANON_KEY").context("SUPABASE_ANON_KEY is required")?;
        let supabase_jwks_url = std::env::var("SUPABASE_JWKS_URL")
            .unwrap_or_else(|_| format!("{}/auth/v1/.well-known/jwks.json", supabase_url.trim_end_matches('/')));
        let port = std::env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(3000);
        Ok(Self {
            database_url,
            redis_url,
            supabase_url,
            supabase_anon_key,
            supabase_jwks_url,
            port,
        })
    }
}
