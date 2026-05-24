//! Reconciliation loop — the heart of the controller.
//!
//! Every `RECONCILE_INTERVAL_SECONDS` seconds:
//!   1. Load manifest from `MANIFEST_PATH`.
//!   2. Fetch SST stack outputs (Lambda ARNs).
//!   3. Resolve each manifest entry → `ResolvedRouteEntry`.
//!   4. Diff against current Valkey state under `apr:route:*`.
//!   5. SET added/changed routes, DEL removed routes.
//!   6. If anything changed, PUBLISH `apr:config-bump`.
//!   7. Update the in-memory cache so the admin API can serve reads
//!      without re-touching Valkey.
//!
//! Single-replica: the Deployment's `strategy: Recreate` guarantees only
//! one of these is running at a time. No leader election needed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use tokio::sync::RwLock;

use crate::controller::manifest_loader;
use crate::controller::pubsub;
use crate::controller::sst_outputs::SstOutputsFetcher;
use crate::controller::types::{ResolvedRouteEntry, RouteEntry, RouteMode};

/// Snapshot of the last reconcile cycle. Held behind an `RwLock` so the
/// admin API can read it concurrently with reconciles.
#[derive(Clone, Debug, Default)]
pub struct ReconcileSnapshot {
    pub routes: Vec<ResolvedRouteEntry>,
    pub last_reconcile_at: Option<DateTime<Utc>>,
    pub last_reconcile_status: ReconcileStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReconcileStatus {
    /// Reconcile has never run yet.
    #[default]
    Pending,
    /// Last cycle completed without error.
    Ok,
    /// Last cycle errored — see logs.
    Error,
}

impl ReconcileStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

/// Result of comparing desired vs current state.
#[derive(Debug, PartialEq, Eq)]
pub struct ReconcileDiff {
    /// Keys to write (added or changed).
    pub upserts: Vec<ResolvedRouteEntry>,
    /// Keys to delete (present in current, absent in desired).
    pub deletes: Vec<String>,
}

impl ReconcileDiff {
    pub fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.deletes.is_empty()
    }
}

/// Compute the diff between desired routes and currently-present Valkey
/// keys. Pure function: easy to unit-test.
///
/// `current_keys` is the set of `apr:route:*` keys currently present in
/// Valkey. `current_payloads` is the parsed JSON for each — used to decide
/// whether an upsert is a real change (different payload) or a no-op
/// (same payload, skip the write).
pub fn compute_diff(
    desired: &[ResolvedRouteEntry],
    current_keys: &HashSet<String>,
    current_payloads: &HashMap<String, ResolvedRouteEntry>,
) -> ReconcileDiff {
    let mut upserts = Vec::new();
    let mut desired_keys = HashSet::with_capacity(desired.len());

    for entry in desired {
        let key = entry.valkey_key();
        desired_keys.insert(key.clone());
        match current_payloads.get(&key) {
            Some(existing) if existing == entry => {
                // No change; skip.
            }
            _ => upserts.push(entry.clone()),
        }
    }

    let deletes: Vec<String> = current_keys
        .iter()
        .filter(|k| !desired_keys.contains(*k))
        .cloned()
        .collect();

    ReconcileDiff { upserts, deletes }
}

/// Resolve `RouteEntry` → `ResolvedRouteEntry` by looking up `lambdaOutputKey`
/// in the SST outputs map. For `proxy`/`cache` modes with a missing key,
/// emits a warning and produces an unresolved entry (lambda_arn = None) —
/// the data plane will treat that route as unhealthy until SST deploys it.
pub fn resolve_routes(
    manifest: Vec<RouteEntry>,
    sst_outputs: &HashMap<String, String>,
) -> Vec<ResolvedRouteEntry> {
    let mut resolved = Vec::with_capacity(manifest.len());
    for entry in manifest {
        let lambda_arn = match (&entry.mode, &entry.lambda_output_key) {
            (RouteMode::Implement, _) => None,
            (_, Some(key)) => match sst_outputs.get(key) {
                Some(arn) => Some(arn.clone()),
                None => {
                    tracing::warn!(
                        method = %entry.method,
                        path = %entry.path,
                        lambda_output_key = %key,
                        "SST outputs missing key for proxy/cache route; route will be unhealthy until SST deploys it"
                    );
                    None
                }
            },
            (_, None) => {
                tracing::warn!(
                    method = %entry.method,
                    path = %entry.path,
                    mode = ?entry.mode,
                    "manifest route in proxy/cache mode has no lambdaOutputKey; route will be unhealthy"
                );
                None
            }
        };
        resolved.push(ResolvedRouteEntry { entry, lambda_arn });
    }
    resolved
}

/// Read every `apr:route:*` key currently in Valkey, returning the set of
/// keys + parsed payloads. Uses `SCAN` (not `KEYS`) for production safety.
pub async fn read_current_state(
    conn: &mut MultiplexedConnection,
) -> Result<(HashSet<String>, HashMap<String, ResolvedRouteEntry>)> {
    let mut keys: HashSet<String> = HashSet::new();
    let mut iter: redis::AsyncIter<'_, String> = conn
        .scan_match("apr:route:*")
        .await
        .context("SCAN apr:route:* failed")?;
    while let Some(key) = futures::StreamExt::next(&mut iter).await {
        keys.insert(key);
    }
    drop(iter);

    let mut payloads = HashMap::with_capacity(keys.len());
    for key in &keys {
        let raw: Option<String> = conn
            .get(key)
            .await
            .with_context(|| format!("GET {} failed", key))?;
        if let Some(raw) = raw {
            match serde_json::from_str::<ResolvedRouteEntry>(&raw) {
                Ok(entry) => {
                    payloads.insert(key.clone(), entry);
                }
                Err(err) => {
                    // Bad payload: log + treat as missing so it gets rewritten
                    // on the next upsert pass.
                    tracing::warn!(key = %key, error = %err, "failed to parse existing apr:route entry; will rewrite");
                }
            }
        }
    }
    Ok((keys, payloads))
}

/// Apply a diff to Valkey: SET upserts, DEL deletes, then PUBLISH
/// `apr:config-bump` if anything changed.
pub async fn apply_diff(conn: &mut MultiplexedConnection, diff: &ReconcileDiff) -> Result<()> {
    for entry in &diff.upserts {
        let key = entry.valkey_key();
        let payload = serde_json::to_string(entry).context("serialize resolved route")?;
        let _: () = conn
            .set(&key, &payload)
            .await
            .with_context(|| format!("SET {} failed", key))?;
    }
    for key in &diff.deletes {
        let _: () = conn
            .del(key)
            .await
            .with_context(|| format!("DEL {} failed", key))?;
    }
    if !diff.is_empty() {
        pubsub::publish_config_bump(conn).await?;
    }
    Ok(())
}

/// Single reconcile cycle. Pure orchestration on top of the helpers above
/// so the loop body stays readable.
pub async fn reconcile_once(
    manifest_path: &str,
    sst_fetcher: &dyn SstOutputsFetcher,
    conn: &mut MultiplexedConnection,
) -> Result<Vec<ResolvedRouteEntry>> {
    let manifest = manifest_loader::load_from_file(manifest_path)
        .await
        .context("manifest load failed")?;
    let sst_outputs = sst_fetcher.fetch().await.context("SST outputs fetch failed")?;
    let resolved = resolve_routes(manifest, &sst_outputs);

    let (current_keys, current_payloads) = read_current_state(conn).await?;
    let diff = compute_diff(&resolved, &current_keys, &current_payloads);

    tracing::info!(
        total = resolved.len(),
        upserts = diff.upserts.len(),
        deletes = diff.deletes.len(),
        "Reconciled {} routes, {} changed, {} removed.",
        resolved.len(),
        diff.upserts.len(),
        diff.deletes.len()
    );

    apply_diff(conn, &diff).await?;
    Ok(resolved)
}

/// Long-running reconcile loop. Cooperatively cancellable via the shutdown
/// receiver — when it fires, the loop exits cleanly without writing a
/// partial cycle.
pub async fn run_loop(
    manifest_path: String,
    sst_fetcher: Arc<dyn SstOutputsFetcher>,
    redis_client: redis::Client,
    interval: Duration,
    snapshot: Arc<RwLock<ReconcileSnapshot>>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(interval);
    // First tick fires immediately so the controller's route table is
    // populated on boot rather than after the first interval.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let mut conn = match redis_client.get_multiplexed_async_connection().await {
                    Ok(c) => c,
                    Err(err) => {
                        tracing::error!(error = %err, "redis connection failed; will retry next tick");
                        let mut snap = snapshot.write().await;
                        snap.last_reconcile_at = Some(Utc::now());
                        snap.last_reconcile_status = ReconcileStatus::Error;
                        continue;
                    }
                };
                match reconcile_once(&manifest_path, sst_fetcher.as_ref(), &mut conn).await {
                    Ok(routes) => {
                        let mut snap = snapshot.write().await;
                        snap.routes = routes;
                        snap.last_reconcile_at = Some(Utc::now());
                        snap.last_reconcile_status = ReconcileStatus::Ok;
                    }
                    Err(err) => {
                        tracing::error!(error = ?err, "reconcile cycle failed");
                        let mut snap = snapshot.write().await;
                        snap.last_reconcile_at = Some(Utc::now());
                        snap.last_reconcile_status = ReconcileStatus::Error;
                    }
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("reconcile loop shutting down");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::types::{
        AuthClass, CacheConfig, ImplementConfig, RateLimitConfig, ResolvedRouteEntry, RouteEntry, RouteMode,
    };

    fn entry(method: &str, path: &str, mode: RouteMode, lambda_key: Option<&str>) -> RouteEntry {
        RouteEntry {
            path: path.to_string(),
            method: method.to_string(),
            auth: AuthClass::User,
            idempotent: true,
            mode,
            rate_limit: RateLimitConfig {
                per_token: 100,
                window_seconds: 60,
            },
            cache: None,
            implement: None,
            lambda_output_key: lambda_key.map(|s| s.to_string()),
            schema_ref: "X".to_string(),
        }
    }

    fn resolved(entry: RouteEntry, arn: Option<&str>) -> ResolvedRouteEntry {
        ResolvedRouteEntry {
            entry,
            lambda_arn: arn.map(|s| s.to_string()),
        }
    }

    #[test]
    fn resolves_proxy_route_when_sst_output_present() {
        let manifest = vec![entry("GET", "/foo", RouteMode::Proxy, Some("FooArn"))];
        let mut sst = HashMap::new();
        sst.insert("FooArn".to_string(), "arn:aws:lambda:us-east-1:1:function:foo".to_string());

        let out = resolve_routes(manifest, &sst);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].lambda_arn.as_deref(), Some("arn:aws:lambda:us-east-1:1:function:foo"));
    }

    #[test]
    fn proxy_route_missing_from_sst_outputs_resolves_to_none() {
        let manifest = vec![entry("GET", "/foo", RouteMode::Proxy, Some("MissingArn"))];
        let sst = HashMap::new();
        let out = resolve_routes(manifest, &sst);
        assert_eq!(out.len(), 1);
        assert!(out[0].lambda_arn.is_none(), "missing SST key should leave lambda_arn None");
    }

    #[test]
    fn implement_route_ignores_lambda_output_key() {
        let mut e = entry("GET", "/profile", RouteMode::Implement, Some("Whatever"));
        e.implement = Some(ImplementConfig {
            rust_handler: "profile".to_string(),
        });
        let mut sst = HashMap::new();
        sst.insert("Whatever".to_string(), "arn:1".to_string());
        let out = resolve_routes(vec![e], &sst);
        assert!(out[0].lambda_arn.is_none(), "implement mode never resolves to a Lambda ARN");
    }

    #[test]
    fn diff_detects_add_change_remove() {
        let want_a = resolved(entry("GET", "/a", RouteMode::Proxy, Some("AArn")), Some("arn:a-v1"));
        let want_b = resolved(entry("GET", "/b", RouteMode::Proxy, Some("BArn")), Some("arn:b-v2"));
        let desired = vec![want_a.clone(), want_b.clone()];

        let mut current_keys: HashSet<String> = HashSet::new();
        current_keys.insert(want_a.valkey_key());
        // /b is currently at v1, will be changed to v2
        let mut want_b_v1 = want_b.clone();
        want_b_v1.lambda_arn = Some("arn:b-v1".to_string());
        current_keys.insert(want_b.valkey_key());
        // /c is currently present but not desired → should be deleted
        let want_c = resolved(entry("GET", "/c", RouteMode::Proxy, Some("CArn")), Some("arn:c"));
        current_keys.insert(want_c.valkey_key());

        let mut payloads = HashMap::new();
        payloads.insert(want_a.valkey_key(), want_a.clone());
        payloads.insert(want_b.valkey_key(), want_b_v1);
        payloads.insert(want_c.valkey_key(), want_c.clone());

        let diff = compute_diff(&desired, &current_keys, &payloads);
        assert_eq!(diff.upserts.len(), 1);
        assert_eq!(diff.upserts[0].entry.path, "/b");
        assert_eq!(diff.deletes, vec![want_c.valkey_key()]);
    }

    #[test]
    fn diff_is_empty_when_state_matches() {
        let want = resolved(entry("GET", "/x", RouteMode::Proxy, Some("XArn")), Some("arn:x"));
        let desired = vec![want.clone()];
        let mut current_keys = HashSet::new();
        current_keys.insert(want.valkey_key());
        let mut payloads = HashMap::new();
        payloads.insert(want.valkey_key(), want.clone());
        let diff = compute_diff(&desired, &current_keys, &payloads);
        assert!(diff.is_empty());
    }

    #[test]
    fn cache_mode_uses_lambda_output_key_too() {
        let mut e = entry("GET", "/cached", RouteMode::Cache, Some("CachedArn"));
        e.cache = Some(CacheConfig {
            ttl_seconds: 60,
            swr_seconds: 30,
            key_template: vec!["user:{auth.sub}".into()],
        });
        let mut sst = HashMap::new();
        sst.insert("CachedArn".to_string(), "arn:cached".to_string());
        let out = resolve_routes(vec![e], &sst);
        assert_eq!(out[0].lambda_arn.as_deref(), Some("arn:cached"));
    }
}
