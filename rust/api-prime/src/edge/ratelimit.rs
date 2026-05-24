//! Sliding-window rate limiting backed by Valkey.
//!
//! Algorithm: per-subject-per-route sorted set at `apr:ratelimit:<sub>:<route_hash>`.
//! Each request adds an entry scored by epoch-ms, then trims entries
//! older than the window and ZCARDs the remainder. Count > limit ⇒ 429.
//!
//! Same algorithm + keyspace today's Hono middleware uses (ADR-017
//! §"Auth + rate-limit at the edge") so there is no data migration —
//! the only change is where enforcement runs.
//!
//! On Valkey unavailability we **fail open**: warn + allow. Failing
//! closed would 5xx every request, which is worse than letting them
//! through with a slightly larger blast radius for the short window
//! before Valkey recovers. ADR review approved this trade-off.

use std::time::{SystemTime, UNIX_EPOCH};

use redis::AsyncCommands;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::edge::auth::EdgeAuthContext;
use crate::edge::types::{RateLimitConfig, RouteEntry};
use crate::error::AppError;

pub struct RateLimiter {
    client: redis::Client,
}

impl RateLimiter {
    pub fn new(client: redis::Client) -> Self {
        Self { client }
    }

    pub async fn check(&self, route: &RouteEntry, auth: &EdgeAuthContext) -> Result<RateLimitOutcome, AppError> {
        let cfg = &route.rate_limit;
        let key = build_key(auth, route);
        let now_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_millis() as u64,
            Err(_) => return Ok(RateLimitOutcome::Allowed { remaining: cfg.per_token }),
        };
        let window_ms = u64::from(cfg.window_seconds) * 1000;
        let cutoff = now_ms.saturating_sub(window_ms);

        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "rate-limit: Valkey unreachable, failing open");
                return Ok(RateLimitOutcome::Allowed { remaining: cfg.per_token });
            }
        };

        let entry = format!("{}:{}", now_ms, uuid::Uuid::new_v4());
        let res: Result<usize, redis::RedisError> = redis::pipe()
            .atomic()
            .cmd("ZREMRANGEBYSCORE")
            .arg(&key)
            .arg(0)
            .arg(cutoff)
            .ignore()
            .zadd(&key, &entry, now_ms)
            .ignore()
            .zcard(&key)
            .expire(&key, cfg.window_seconds as i64 + 1)
            .ignore()
            .query_async(&mut conn)
            .await;

        let count = match res {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "rate-limit: Valkey op failed, failing open");
                return Ok(RateLimitOutcome::Allowed { remaining: cfg.per_token });
            }
        };

        let limit = cfg.per_token as usize;
        if count > limit {
            let _: Result<i64, _> = conn.zrem(&key, &entry).await;
            Ok(RateLimitOutcome::Throttled {
                limit: cfg.per_token,
                window_seconds: cfg.window_seconds,
            })
        } else {
            Ok(RateLimitOutcome::Allowed {
                remaining: (limit - count) as u32,
            })
        }
    }
}

#[derive(Debug, Clone)]
pub enum RateLimitOutcome {
    Allowed { remaining: u32 },
    Throttled { limit: u32, window_seconds: u32 },
}

impl RateLimitOutcome {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateLimitOutcome::Allowed { .. })
    }
}

pub fn route_hash(route: &RouteEntry) -> String {
    let mut h = Sha256::new();
    h.update(route.method.as_bytes());
    h.update(b" ");
    h.update(route.path.as_bytes());
    hex::encode(&h.finalize()[..4])
}

fn build_key(auth: &EdgeAuthContext, route: &RouteEntry) -> String {
    format!("apr:ratelimit:{}:{}", auth.sub, route_hash(route))
}

#[allow(dead_code)]
pub fn default_config() -> RateLimitConfig {
    RateLimitConfig {
        per_token: 100,
        window_seconds: 60,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::auth::AuthKind;
    use crate::edge::types::{AuthRequirement, RouteMode};

    fn route() -> RouteEntry {
        RouteEntry {
            path: "/foo/:id".into(),
            method: "GET".into(),
            auth: AuthRequirement::User,
            idempotent: true,
            mode: RouteMode::Proxy,
            rate_limit: RateLimitConfig {
                per_token: 5,
                window_seconds: 60,
            },
            cache: None,
            implement: None,
            lambda_arn: Some("arn".into()),
            schema_ref: None,
        }
    }

    #[test]
    fn key_includes_sub_and_route_hash() {
        let auth = EdgeAuthContext {
            sub: "user-abc".into(),
            kind: AuthKind::User { user_id: "user-abc".into() },
            raw_jwt: None,
        };
        let key = build_key(&auth, &route());
        assert!(key.starts_with("apr:ratelimit:user-abc:"));
        let parts: Vec<&str> = key.split(':').collect();
        assert_eq!(parts.last().unwrap().len(), 8);
    }

    #[test]
    fn route_hash_is_stable() {
        assert_eq!(route_hash(&route()), route_hash(&route()));
    }

    #[test]
    fn outcome_is_allowed_flag() {
        assert!(RateLimitOutcome::Allowed { remaining: 1 }.is_allowed());
        assert!(!RateLimitOutcome::Throttled { limit: 1, window_seconds: 1 }.is_allowed());
    }
}
