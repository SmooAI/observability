//! api-prime-controller: control-plane binary for the api-prime edge mesh.
//!
//! See ADR-017 (`docs/Decisions/ADR-017-API-Prime-Programmable-Edge.md` in
//! the smooai repo) for the full architecture. This binary:
//!
//!   - Runs the reconcile loop every `RECONCILE_INTERVAL_SECONDS`
//!     (default 30s): loads manifest from `MANIFEST_PATH`, fetches SST
//!     stack outputs from S3, resolves Lambda ARNs, diffs against Valkey,
//!     and writes changes — publishing `apr:config-bump` if anything
//!     moved.
//!   - Serves the admin API at `/admin/v1/*` (route inspection, mode
//!     flipping, manual invalidation, OpenAPI).
//!   - Serves the internal API at `/internal/v1/cache/invalidate` for
//!     HMAC-authenticated mutation Lambdas.
//!
//! Single-replica: the Deployment uses `strategy: Recreate`. No leader
//! election or multi-instance coordination in this binary.
//!
//! Graceful shutdown: SIGTERM stops accepting new connections, drains the
//! reconcile loop, then exits. Critical for k8s rolling updates.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::DefaultBodyLimit;
use smooai_api_prime::controller::{
    admin,
    internal::ControllerState,
    reconcile::{ReconcileSnapshot, ReconcileStatus},
    sst_outputs::{S3OutputsFetcher, SstOutputsFetcher},
};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{watch, RwLock};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cfg = ControllerConfig::from_env()?;
    tracing::info!(
        port = cfg.port,
        manifest_path = %cfg.manifest_path,
        reconcile_interval_secs = cfg.reconcile_interval.as_secs(),
        admin_dev_bypass = cfg.admin_dev_bypass,
        "starting api-prime-controller"
    );

    // Redis client (cheap; connections are lazily established).
    let redis_client =
        redis::Client::open(cfg.redis_url.as_str()).context("failed to construct redis client")?;

    // SST outputs fetcher. We pull AWS config from the default chain so
    // IRSA-provided creds in the pod just work.
    let sst_fetcher: Arc<dyn SstOutputsFetcher> = {
        let aws_cfg = aws_config::from_env().load().await;
        let s3 = aws_sdk_s3::Client::new(&aws_cfg);
        Arc::new(S3OutputsFetcher::from_stage(
            s3,
            cfg.sst_state_bucket.clone(),
            &cfg.stage,
        ))
    };

    // Shared snapshot, written by reconcile loop + read by admin handlers.
    let snapshot = Arc::new(RwLock::new(ReconcileSnapshot {
        last_reconcile_status: ReconcileStatus::Pending,
        ..Default::default()
    }));

    // Spawn reconcile loop.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let reconcile_handle = {
        let snapshot = snapshot.clone();
        let redis_client = redis_client.clone();
        let manifest_path = cfg.manifest_path.clone();
        let sst_fetcher = sst_fetcher.clone();
        let interval = cfg.reconcile_interval;
        let shutdown_rx = shutdown_rx.clone();
        tokio::spawn(async move {
            smooai_api_prime::controller::reconcile::run_loop(
                manifest_path,
                sst_fetcher,
                redis_client,
                interval,
                snapshot,
                shutdown_rx,
            )
            .await;
        })
    };

    // Build axum app.
    let state = ControllerState {
        redis: redis_client,
        edge_attest_secret: Arc::new(cfg.edge_attest_secret),
        snapshot: snapshot.clone(),
        started_at: chrono::Utc::now(),
        admin_dev_bypass: cfg.admin_dev_bypass,
    };

    // Admin + internal routes are bundled inside `admin::router` so we
    // build one stateful router and don't have to juggle state types.
    let app = admin::router(state)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr =
        format!("0.0.0.0:{}", cfg.port).parse().context("invalid bind address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {}", addr))?;
    tracing::info!("listening on {}", addr);

    // Graceful shutdown wires SIGTERM + SIGINT → drain reconcile loop +
    // close listener.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal(shutdown_tx))
        .await?;

    // Wait for reconcile loop to drain.
    if let Err(err) = reconcile_handle.await {
        tracing::warn!(error = ?err, "reconcile task join error");
    }
    tracing::info!("api-prime-controller exiting cleanly");
    Ok(())
}

async fn shutdown_signal(tx: watch::Sender<bool>) {
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => tracing::info!("SIGTERM received; draining"),
        _ = int.recv() => tracing::info!("SIGINT received; draining"),
    }
    let _ = tx.send(true);
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry().with(filter).with(fmt::layer().json()).init();
}

struct ControllerConfig {
    port: u16,
    redis_url: String,
    manifest_path: String,
    sst_state_bucket: String,
    stage: String,
    reconcile_interval: Duration,
    edge_attest_secret: String,
    /// Disables the `X-Smoo-Admin-Authenticated` header gate. Intended for
    /// local development only; production sets this to `false`.
    admin_dev_bypass: bool,
}

impl ControllerConfig {
    fn from_env() -> Result<Self> {
        let port = std::env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8080);
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let manifest_path = std::env::var("MANIFEST_PATH")
            .unwrap_or_else(|_| "/etc/api-prime/manifest.json".to_string());
        let sst_state_bucket = std::env::var("SST_STATE_BUCKET").context(
            "SST_STATE_BUCKET is required so the controller knows where to read SST outputs from",
        )?;
        let stage = std::env::var("STAGE").context("STAGE is required (e.g., production, brentrager)")?;
        let reconcile_interval_secs = std::env::var("RECONCILE_INTERVAL_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);
        let edge_attest_secret = std::env::var("EDGE_ATTEST_SECRET").context(
            "EDGE_ATTEST_SECRET is required for /internal/v1/cache/invalidate HMAC verification",
        )?;
        let admin_dev_bypass = std::env::var("ADMIN_DEV_BYPASS")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        Ok(Self {
            port,
            redis_url,
            manifest_path,
            sst_state_bucket,
            stage,
            reconcile_interval: Duration::from_secs(reconcile_interval_secs.max(1)),
            edge_attest_secret,
            admin_dev_bypass,
        })
    }
}
