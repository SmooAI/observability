//! Cache mode — L1 in-proc (moka) + L2 Valkey + SWR semantics.
//!
//! For cache-mode routes the dispatcher calls [`EdgeCache::get`] then
//! [`EdgeCache::put`] around a fresh-fetch closure. The cache layer:
//!
//! - **HIT**: returns the L1 entry if fresh (`now < ttl_at`).
//! - **HIT-L2**: misses L1, hits L2, repopulates L1.
//! - **STALE**: returns the cached entry but the dispatcher spawns a
//!   background refresh (stale-while-revalidate window: `ttl_at < now < swr_at`).
//! - **MISS**: synchronously fetches + stores + returns.
//!
//! Cache key composition uses `route.cache.keyTemplate`. The composed
//! fragments are joined with NUL (`\x00`) and SHA-256'd — the digest
//! is the canonical L2 key. Each fragment is also a tag, stored in
//! `apr:cache:tag:<tag>` SETs so the controller can DEL cache keys by
//! tag on invalidation.
//!
//! L1 is best-effort: a per-pod pubsub subscriber drops entries on
//! `apr:invalidate`. If a pod misses a message, the entry expires at
//! its TTL — worst case, not a correctness bug. Valkey L2 is the
//! source of truth.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use moka::future::Cache as MokaCache;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::edge::auth::EdgeAuthContext;
use crate::edge::types::{CacheConfig, RouteEntry};

/// Cached response body + framing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub is_base64_encoded: bool,
    pub cached_at: u64,
    pub ttl_at: u64,
    pub swr_at: u64,
}

impl CachedResponse {
    pub fn freshness(&self) -> Freshness {
        let now = now_secs();
        if now < self.ttl_at {
            Freshness::Fresh
        } else if now < self.swr_at {
            Freshness::Stale
        } else {
            Freshness::Expired
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
    Expired,
}

/// Status reported back to the dispatcher for debug headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    Hit,
    Miss,
    Stale,
}

impl CacheStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CacheStatus::Hit => "HIT",
            CacheStatus::Miss => "MISS",
            CacheStatus::Stale => "STALE",
        }
    }
}

pub struct EdgeCache {
    l1: MokaCache<String, CachedResponse>,
    l2: redis::Client,
    /// Per-pod tag → set-of-cache-keys index for L1 invalidation by tag.
    tag_index: DashMap<String, dashmap::DashSet<String>>,
    /// Guards background refresh dedup — at most one in-flight refresh
    /// per cache key so a stampede on a stale entry doesn't fan out
    /// hundreds of Lambda invokes.
    refresh_locks: DashMap<String, Arc<Mutex<()>>>,
}

impl EdgeCache {
    pub fn new(l2: redis::Client, l1_max_entries: u64) -> Self {
        Self {
            l1: MokaCache::builder().max_capacity(l1_max_entries).build(),
            l2,
            tag_index: DashMap::new(),
            refresh_locks: DashMap::new(),
        }
    }

    /// Resolve `route.cache.keyTemplate` against the request context.
    pub fn compose_key(
        cfg: &CacheConfig,
        route: &RouteEntry,
        auth: &EdgeAuthContext,
        path_params: &std::collections::HashMap<String, String>,
    ) -> Vec<String> {
        let mut out = Vec::with_capacity(cfg.key_template.len() + 1);
        // Route descriptor itself is implicitly a tag so different routes
        // can't collide on identical user/org fragments.
        out.push(format!("route:{}:{}", route.method.to_uppercase(), route.path));
        for fragment in &cfg.key_template {
            out.push(resolve_fragment(fragment, auth, path_params));
        }
        out
    }

    /// SHA-256 the composed fragments (NUL-joined) and hex-encode.
    pub fn canonical_key(fragments: &[String]) -> String {
        let mut h = Sha256::new();
        for (i, f) in fragments.iter().enumerate() {
            if i > 0 {
                h.update(b"\x00");
            }
            h.update(f.as_bytes());
        }
        hex::encode(h.finalize())
    }

    /// Look up the L1 → L2 chain.
    pub async fn get(&self, canonical: &str) -> Option<(CachedResponse, CacheTier)> {
        if let Some(entry) = self.l1.get(canonical).await {
            return Some((entry, CacheTier::L1));
        }
        let key = format!("apr:cache:{canonical}");
        let mut conn = self.l2.get_multiplexed_async_connection().await.ok()?;
        let raw: Option<String> = conn.get(&key).await.ok()?;
        let raw = raw?;
        match serde_json::from_str::<CachedResponse>(&raw) {
            Ok(entry) => {
                self.l1.insert(canonical.to_string(), entry.clone()).await;
                Some((entry, CacheTier::L2))
            }
            Err(e) => {
                warn!(error = %e, key = %key, "malformed L2 cache entry, dropping");
                let _: Result<i64, _> = conn.del(&key).await;
                None
            }
        }
    }

    /// Write to L1 + L2 + tag index.
    pub async fn put(&self, canonical: &str, fragments: &[String], entry: CachedResponse) {
        self.l1.insert(canonical.to_string(), entry.clone()).await;
        let key = format!("apr:cache:{canonical}");
        let body = match serde_json::to_string(&entry) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "cannot serialize cache entry, skipping L2 write");
                return;
            }
        };
        let ttl_secs = entry.swr_at.saturating_sub(now_secs()).max(1);
        match self.l2.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                let _: Result<(), redis::RedisError> = redis::pipe()
                    .atomic()
                    .cmd("SETEX")
                    .arg(&key)
                    .arg(ttl_secs as usize)
                    .arg(&body)
                    .ignore()
                    .query_async(&mut conn)
                    .await;
                for tag in fragments {
                    let tag_key = format!("apr:cache:tag:{tag}");
                    let _: Result<(), redis::RedisError> = redis::pipe()
                        .atomic()
                        .sadd(&tag_key, canonical)
                        .ignore()
                        .expire(&tag_key, (ttl_secs as i64).max(60))
                        .ignore()
                        .query_async(&mut conn)
                        .await;
                }
            }
            Err(e) => warn!(error = %e, "L2 cache write failed (entry lives in L1 only)"),
        }
        for tag in fragments {
            self.tag_index
                .entry(tag.clone())
                .or_default()
                .insert(canonical.to_string());
        }
    }

    /// Drop a tag's worth of L1 entries.
    pub async fn invalidate_tags(&self, tags: &[String]) {
        for tag in tags {
            if let Some((_, keys)) = self.tag_index.remove(tag) {
                for k in keys.iter() {
                    self.l1.invalidate(k.key()).await;
                }
            }
        }
    }

    /// Drop the entire L1 — used on `apr:config-bump`.
    pub async fn clear_l1(&self) {
        self.l1.invalidate_all();
        self.tag_index.clear();
    }

    /// Per-key mutex to serialize background refreshes.
    pub fn refresh_lock(&self, key: &str) -> Arc<Mutex<()>> {
        Arc::clone(
            &self
                .refresh_locks
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CacheTier {
    L1,
    L2,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

pub fn compute_expiry(cfg: &CacheConfig) -> (u64, u64, u64) {
    let now = now_secs();
    let ttl_at = now + u64::from(cfg.ttl_seconds);
    let swr_at = ttl_at + u64::from(cfg.swr_seconds);
    (now, ttl_at, swr_at)
}

fn resolve_fragment(
    template: &str,
    auth: &EdgeAuthContext,
    path_params: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut placeholder = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                placeholder.push(c);
            }
            let val = match placeholder.as_str() {
                "auth.sub" => auth.sub.clone(),
                "auth.kind" => auth.kind.as_str().to_string(),
                other => other
                    .strip_prefix(':')
                    .and_then(|n| path_params.get(n).cloned())
                    .unwrap_or_else(|| "_".to_string()),
            };
            for ch in val.chars() {
                if ch == ':' || ch == '\x00' {
                    debug!(value = %val, "stripping separator from cache fragment value");
                    out.push('_');
                } else {
                    out.push(ch);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::auth::AuthKind;
    use crate::edge::types::{AuthRequirement, RateLimitConfig, RouteMode};
    use std::collections::HashMap;

    fn auth() -> EdgeAuthContext {
        EdgeAuthContext {
            sub: "user-1".into(),
            kind: AuthKind::User { user_id: "user-1".into() },
            raw_jwt: None,
        }
    }

    fn cfg() -> CacheConfig {
        CacheConfig {
            ttl_seconds: 60,
            swr_seconds: 60,
            key_template: vec!["user:{auth.sub}".into(), "org:{:org_id}".into()],
        }
    }

    fn route() -> RouteEntry {
        RouteEntry {
            path: "/orgs/:org_id/items".into(),
            method: "GET".into(),
            auth: AuthRequirement::User,
            idempotent: true,
            mode: RouteMode::Cache,
            rate_limit: RateLimitConfig {
                per_token: 100,
                window_seconds: 60,
            },
            cache: Some(cfg()),
            implement: None,
            lambda_arn: Some("arn".into()),
            schema_ref: None,
        }
    }

    #[test]
    fn compose_key_resolves_auth_and_path() {
        let mut params = HashMap::new();
        params.insert("org_id".into(), "abc".into());
        let fragments = EdgeCache::compose_key(&cfg(), &route(), &auth(), &params);
        assert_eq!(fragments.len(), 3);
        assert_eq!(fragments[0], "route:GET:/orgs/:org_id/items");
        assert_eq!(fragments[1], "user:user-1");
        assert_eq!(fragments[2], "org:abc");
    }

    #[test]
    fn canonical_key_is_deterministic_and_distinct() {
        let a = EdgeCache::canonical_key(&["user:1".into(), "org:x".into()]);
        let b = EdgeCache::canonical_key(&["user:1".into(), "org:x".into()]);
        let c = EdgeCache::canonical_key(&["user:1".into(), "org:y".into()]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn freshness_transitions() {
        let now = now_secs();
        let entry = CachedResponse {
            status: 200,
            headers: vec![],
            body: "{}".into(),
            is_base64_encoded: false,
            cached_at: now,
            ttl_at: now + 10,
            swr_at: now + 20,
        };
        assert_eq!(entry.freshness(), Freshness::Fresh);

        let stale = CachedResponse { ttl_at: now - 5, swr_at: now + 5, ..entry.clone() };
        assert_eq!(stale.freshness(), Freshness::Stale);

        let expired = CachedResponse { ttl_at: now - 100, swr_at: now - 50, ..entry };
        assert_eq!(expired.freshness(), Freshness::Expired);
    }

    #[test]
    fn fragment_resolves_known_placeholders_and_strips_separators() {
        let mut params = HashMap::new();
        params.insert("org_id".into(), "ab:cd".into());
        let s = resolve_fragment("org:{:org_id}", &auth(), &params);
        assert_eq!(s, "org:ab_cd");

        let s = resolve_fragment("user:{auth.sub}", &auth(), &params);
        assert_eq!(s, "user:user-1");

        let s = resolve_fragment("x:{nope}", &auth(), &params);
        assert_eq!(s, "x:_");
    }
}
