//! In-process route table cache.
//!
//! Source of truth lives in Valkey at `apr:route:<METHOD>:<path>` (one
//! JSON-encoded [`RouteEntry`] per key, written by the controller). The
//! data plane:
//!
//! 1. On startup: `SCAN apr:route:*` → parse → publish into a
//!    `RwLock<Arc<Snapshot>>`.
//! 2. On every request: clone the `Arc<Snapshot>` and lookup without
//!    holding the lock through dispatch.
//! 3. On `apr:config-bump`: re-SCAN and atomically swap the snapshot.
//!
//! Fallback: if `LOCAL_MANIFEST_PATH` is set, load from disk + watch
//! mtime every 5s. Used by local dev + integration harness so the
//! controller doesn't have to be running.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use redis::AsyncCommands;
use regex::Regex;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::edge::types::RouteEntry;

/// Compiled view of a single route entry — original entry + a regex for
/// path matching.
pub struct CompiledRoute {
    pub entry: RouteEntry,
    matcher: Regex,
    param_names: Vec<String>,
}

impl CompiledRoute {
    fn compile(entry: RouteEntry) -> Result<Self> {
        let (matcher, param_names) = compile_path_pattern(&entry.path)?;
        Ok(Self {
            entry,
            matcher,
            param_names,
        })
    }

    pub fn match_path(&self, path: &str) -> Option<HashMap<String, String>> {
        let caps = self.matcher.captures(path)?;
        let mut params = HashMap::with_capacity(self.param_names.len());
        for name in &self.param_names {
            if let Some(m) = caps.name(name) {
                params.insert(name.clone(), m.as_str().to_string());
            }
        }
        Some(params)
    }
}

/// Immutable snapshot the request hot path reads.
pub struct Snapshot {
    by_method: HashMap<String, Vec<CompiledRoute>>,
}

impl Snapshot {
    fn empty() -> Self {
        Self { by_method: HashMap::new() }
    }

    fn from_entries(entries: Vec<RouteEntry>) -> Self {
        let mut by_method: HashMap<String, Vec<CompiledRoute>> = HashMap::new();
        for entry in entries {
            let method = entry.method.to_uppercase();
            match CompiledRoute::compile(entry) {
                Ok(r) => by_method.entry(method).or_default().push(r),
                Err(e) => warn!(error = %e, "skipping route with invalid path pattern"),
            }
        }
        Self { by_method }
    }

    pub fn lookup(&self, method: &str, path: &str) -> Option<(&CompiledRoute, HashMap<String, String>)> {
        let routes = self.by_method.get(&method.to_uppercase())?;
        for r in routes {
            if let Some(params) = r.match_path(path) {
                return Some((r, params));
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.by_method.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct RouteTable {
    inner: RwLock<Arc<Snapshot>>,
    source: RouteSource,
}

#[derive(Clone)]
enum RouteSource {
    Valkey(redis::Client),
    LocalFile(PathBuf),
}

impl RouteTable {
    pub async fn from_valkey(client: redis::Client) -> Result<Arc<Self>> {
        let me = Arc::new(Self {
            inner: RwLock::new(Arc::new(Snapshot::empty())),
            source: RouteSource::Valkey(client),
        });
        me.refresh().await?;
        Ok(me)
    }

    pub async fn from_local_file(path: PathBuf) -> Result<Arc<Self>> {
        let me = Arc::new(Self {
            inner: RwLock::new(Arc::new(Snapshot::empty())),
            source: RouteSource::LocalFile(path.clone()),
        });
        info!(path = %path.display(), "loading route table from local manifest");
        me.refresh().await?;
        let watcher = Arc::clone(&me);
        tokio::spawn(async move {
            let mut last_mtime: Option<SystemTime> = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
                if mtime != last_mtime {
                    last_mtime = mtime;
                    if let Err(e) = watcher.refresh().await {
                        error!(error = %e, "local manifest reload failed");
                    } else {
                        info!("local manifest reloaded");
                    }
                }
            }
        });
        Ok(me)
    }

    /// Atomic Arc clone of the current snapshot — safe to keep across
    /// the entire request without holding the table lock.
    pub async fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&*self.inner.read().await)
    }

    pub async fn refresh(&self) -> Result<()> {
        let entries = match &self.source {
            RouteSource::Valkey(client) => load_from_valkey(client).await?,
            RouteSource::LocalFile(p) => load_from_file(p)?,
        };
        let count = entries.len();
        let new_snapshot = Arc::new(Snapshot::from_entries(entries));
        let total = new_snapshot.len();
        {
            let mut guard = self.inner.write().await;
            *guard = new_snapshot;
        }
        info!(loaded = count, compiled = total, "route table refreshed");
        Ok(())
    }
}

async fn load_from_valkey(client: &redis::Client) -> Result<Vec<RouteEntry>> {
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .context("connect to Valkey for route table SCAN")?;
    let mut cursor: u64 = 0;
    let mut keys: Vec<String> = Vec::new();
    loop {
        let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("apr:route:*")
            .arg("COUNT")
            .arg(500)
            .query_async(&mut conn)
            .await
            .context("SCAN apr:route:*")?;
        keys.extend(batch);
        cursor = next;
        if cursor == 0 {
            break;
        }
    }
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let bodies: Vec<Option<String>> = conn.mget(&keys).await.context("MGET apr:route:*")?;
    let mut entries = Vec::with_capacity(bodies.len());
    for (key, body) in keys.iter().zip(bodies) {
        let Some(body) = body else { continue };
        match serde_json::from_str::<RouteEntry>(&body) {
            Ok(e) => entries.push(e),
            Err(e) => warn!(error = %e, key = %key, "skipping malformed route entry"),
        }
    }
    Ok(entries)
}

fn load_from_file(path: &PathBuf) -> Result<Vec<RouteEntry>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read manifest from {}", path.display()))?;
    if let Ok(arr) = serde_json::from_str::<Vec<RouteEntry>>(&raw) {
        return Ok(arr);
    }
    #[derive(serde::Deserialize)]
    struct Wrapper {
        routes: Vec<RouteEntry>,
    }
    let wrapper: Wrapper = serde_json::from_str(&raw).context("manifest must be array of RouteEntry or {routes: [...]}")?;
    Ok(wrapper.routes)
}

fn compile_path_pattern(template: &str) -> Result<(Regex, Vec<String>)> {
    let mut pattern = String::with_capacity(template.len() + 16);
    pattern.push('^');
    let mut params = Vec::new();
    for segment in template.split('/') {
        if segment.is_empty() {
            continue;
        }
        pattern.push('/');
        if let Some(name) = segment.strip_prefix(':') {
            if name.is_empty() {
                return Err(anyhow!("empty :param in path template {template}"));
            }
            pattern.push_str("(?P<");
            pattern.push_str(name);
            pattern.push_str(">[^/]+)");
            params.push(name.to_string());
        } else {
            pattern.push_str(&regex::escape(segment));
        }
    }
    if pattern == "^" {
        pattern.push('/');
    }
    pattern.push_str("/?$");
    let re = Regex::new(&pattern).with_context(|| format!("compile regex for {template}"))?;
    Ok((re, params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::types::{AuthRequirement, RateLimitConfig, RouteMode};

    fn entry(method: &str, path: &str) -> RouteEntry {
        RouteEntry {
            path: path.to_string(),
            method: method.to_string(),
            auth: AuthRequirement::Public,
            idempotent: true,
            mode: RouteMode::Proxy,
            rate_limit: RateLimitConfig {
                per_token: 100,
                window_seconds: 60,
            },
            cache: None,
            implement: None,
            lambda_arn: Some("arn:fake".to_string()),
            schema_ref: None,
        }
    }

    #[test]
    fn compiles_static_path() {
        let (re, params) = compile_path_pattern("/health/liveness").unwrap();
        assert!(params.is_empty());
        assert!(re.is_match("/health/liveness"));
        assert!(re.is_match("/health/liveness/"));
        assert!(!re.is_match("/health"));
        assert!(!re.is_match("/health/liveness/extra"));
    }

    #[test]
    fn compiles_param_path_and_extracts() {
        let (re, params) = compile_path_pattern("/orgs/:org_id/items/:item_id").unwrap();
        assert_eq!(params, vec!["org_id", "item_id"]);
        let caps = re.captures("/orgs/abc/items/42").unwrap();
        assert_eq!(&caps["org_id"], "abc");
        assert_eq!(&caps["item_id"], "42");
    }

    #[test]
    fn snapshot_lookup_by_method() {
        let snap = Snapshot::from_entries(vec![
            entry("GET", "/orgs/:org_id"),
            entry("POST", "/orgs/:org_id"),
            entry("GET", "/orgs/:org_id/items"),
        ]);
        let (route, params) = snap.lookup("GET", "/orgs/xyz/items").expect("must match");
        assert_eq!(route.entry.path, "/orgs/:org_id/items");
        assert_eq!(params.get("org_id").unwrap(), "xyz");

        assert!(snap.lookup("DELETE", "/orgs/xyz/items").is_none());
    }
}
