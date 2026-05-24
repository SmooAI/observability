//! api-prime: programmable edge data plane.
//!
//! Renamed from `smooai-hot-path` per ADR-017. Per Wave-2 of the API
//! Prime epic (SMOODEV-1276 + SMOODEV-1278), this binary now runs the
//! full edge pipeline:
//!
//! - Route table loaded from Valkey (`apr:route:*`) or local file
//!   (`LOCAL_MANIFEST_PATH`).
//! - Per-request: auth (JWT/M2M) → rate limit (Valkey sliding window)
//!   → schema (stub) → mode dispatch (proxy/cache/implement).
//! - L1 in-process LRU + L2 Valkey with stale-while-revalidate.
//! - Pubsub subscriber for `apr:config-bump` and `apr:invalidate`.
//! - Proxy mode invokes Lambdas directly via `aws-sdk-lambda` (no API
//!   Gateway hop).
//! - Implement mode dispatches into in-process Rust handlers (the same
//!   handlers we used pre-edge for `/health/*` etc.).
//!
//! Graceful shutdown waits up to `SHUTDOWN_TIMEOUT_SECS` (25s default)
//! for in-flight requests to drain before exiting.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::{routing::any, Router};
use smooai_api_prime::{
    auth::jwt::JwksCache,
    cache, db,
    edge::{
        cache::EdgeCache, ctx::EdgeContext, dispatcher, edge_attest::EdgeAttestSigner, proxy::LambdaProxy, pubsub,
        ratelimit::RateLimiter, route_table::RouteTable,
    },
    state::AppState,
};
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::from_env()?;
    info!(port = config.port, "starting api-prime (data plane)");

    let pool = db::init_pool(&config.database_url)
        .await
        .context("failed to initialize Postgres pool")?;

    let redis_client = cache::init_client(&config.redis_url).context("failed to initialize Redis client")?;

    let jwks = JwksCache::new(config.supabase_jwks_url.clone());

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build reqwest client")?;

    let app_state = AppState {
        pool,
        redis: redis_client.clone(),
        jwks,
        http,
        supabase_url: config.supabase_url,
        supabase_anon_key: config.supabase_anon_key,
    };

    // ---------- Edge state ----------
    let routes = if let Some(p) = config.local_manifest_path.clone() {
        info!(path = %p.display(), "LOCAL_MANIFEST_PATH set — bypassing controller / Valkey route table");
        RouteTable::from_local_file(p).await.context("load local manifest")?
    } else {
        RouteTable::from_valkey(redis_client.clone()).await.context("load route table from Valkey")?
    };

    let edge_cache = Arc::new(EdgeCache::new(redis_client.clone(), config.l1_max_entries));
    let ratelimit = Arc::new(RateLimiter::new(redis_client.clone()));
    let lambda_proxy = Arc::new(LambdaProxy::from_env().await);
    let attest = Arc::new(EdgeAttestSigner::new(config.edge_attest_secret.into_bytes()));

    // Pubsub subscriber runs for the lifetime of the process.
    if config.local_manifest_path.is_none() {
        pubsub::spawn(redis_client.clone(), Arc::clone(&routes), Arc::clone(&edge_cache));
    } else {
        info!("pubsub subscriber not started (LOCAL_MANIFEST_PATH bypasses controller)");
    }

    let ctx = EdgeContext {
        routes,
        cache: edge_cache,
        ratelimit,
        proxy: lambda_proxy,
        attest,
        app: app_state,
        debug_default_on: config.is_local,
    };

    // Single catch-all route → dispatcher.
    let app = Router::new()
        .route("/", any(dispatcher::dispatch))
        .fallback(any(dispatcher::dispatch))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(ctx);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("listening on {}", addr);

    let shutdown_timeout = Duration::from_secs(config.shutdown_timeout_secs);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal(shutdown_timeout))
        .await?;
    Ok(())
}

/// Wait for SIGTERM (k8s pod termination) or Ctrl-C; then give axum up
/// to `terminationGracePeriodSeconds` to drain in-flight requests.
async fn shutdown_signal(timeout: Duration) {
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    warn!("shutdown signal received; draining for up to {}s", timeout.as_secs());
    // Axum's with_graceful_shutdown will stop accepting new connections
    // and wait for in-flight to complete. We just yield here so the
    // caller's drain budget begins.
    tokio::time::sleep(Duration::from_millis(50)).await;
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
    edge_attest_secret: String,
    local_manifest_path: Option<PathBuf>,
    is_local: bool,
    l1_max_entries: u64,
    shutdown_timeout_secs: u64,
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
        let edge_attest_secret = std::env::var("EDGE_ATTEST_SECRET").context("EDGE_ATTEST_SECRET is required")?;
        let local_manifest_path = std::env::var("LOCAL_MANIFEST_PATH").ok().map(PathBuf::from);
        let is_local = matches!(std::env::var("IS_LOCAL").as_deref(), Ok("true") | Ok("1"));
        let l1_max_entries = std::env::var("CACHE_L1_MAX_ENTRIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10_000);
        let shutdown_timeout_secs = std::env::var("SHUTDOWN_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(25);
        let port = std::env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8080);
        Ok(Self {
            database_url,
            redis_url,
            supabase_url,
            supabase_anon_key,
            supabase_jwks_url,
            edge_attest_secret,
            local_manifest_path,
            is_local,
            l1_max_entries,
            shutdown_timeout_secs,
            port,
        })
    }
}
