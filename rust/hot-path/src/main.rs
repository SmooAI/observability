//! smooai-hot-path: axum HTTP service serving high-traffic read endpoints.
//!
//! See `README.md` for the full overview. This crate is part of the
//! SMOODEV-1227 scaffold (Phase 5c of the EKS migration plan).
//!
//! Endpoints exposed today:
//! - `GET  /health/liveness`                       — process alive
//! - `GET  /health/readiness`                      — pool + redis reachable
//! - `GET  /v1/profile`                            — Supabase-JWT-authenticated profile read
//! - `POST /v1/auth/sign-in`                       — stub (501), see `handlers::auth`
//! - `GET  /v1/organizations`                      — orgs the user is a member of (+ parent-admin managed)
//! - `GET  /v1/organizations/:org_id/features`     — computed feature set (defaults + products + overrides + internal)
//! - `GET  /v1/organizations/:org_id/products`     — active products with order + stripe_product relations

use std::net::SocketAddr;

use anyhow::Context;
use axum::{
    routing::{get, post},
    Router,
};
use smooai_hot_path::{auth::jwt::JwksCache, cache, db, handlers, state::AppState};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!(port = config.port, "starting smooai-hot-path");

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
        .route("/v1/organizations", get(handlers::organizations::list_organizations))
        .route(
            "/v1/organizations/:org_id/features",
            get(handlers::organization_features::get_organization_features),
        )
        .route(
            "/v1/organizations/:org_id/products",
            get(handlers::organization_products::list_organization_products),
        )
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, app.into_make_service()).await?;
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
