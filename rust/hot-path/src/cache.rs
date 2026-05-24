//! Redis client helpers. Used for read-through caching of `/v1/profile`
//! (key: `profile:{user_id}`, TTL 5min) plus shared cache hits with the
//! existing TS backend that uses the same Redis instance.
//!
//! Sidebar reads (SMOODEV-1238) — orgs/features/products — also flow
//! through here, all with a 60s TTL. The features endpoint is the heaviest
//! of the three per Theia's harness analysis, so the per-(org, user) cache
//! key matters: it gives us a hit on the common "user reloads the
//! dashboard" path without leaking another user's effective feature set.

use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;

pub const PROFILE_TTL_SECS: u64 = 300;

/// SMOODEV-1238 — read-through TTL for the sidebar reads.
/// Short enough that override + product changes propagate within a minute,
/// long enough to absorb the burst of requests a dashboard load produces.
pub const SIDEBAR_TTL_SECS: u64 = 60;

pub fn init_client(redis_url: &str) -> anyhow::Result<redis::Client> {
    let client = redis::Client::open(redis_url)?;
    Ok(client)
}

pub async fn get_connection(client: &redis::Client) -> redis::RedisResult<MultiplexedConnection> {
    client.get_multiplexed_async_connection().await
}

pub fn profile_key(user_id: &str) -> String {
    format!("profile:{}", user_id)
}

pub fn organizations_key(user_id: &str) -> String {
    format!("orgs:{}", user_id)
}

pub fn organization_features_key(org_id: &str, user_id: &str) -> String {
    format!("features:{}:{}", org_id, user_id)
}

pub fn organization_products_key(org_id: &str, user_id: &str) -> String {
    format!("products:{}:{}", org_id, user_id)
}

pub async fn get_string(conn: &mut MultiplexedConnection, key: &str) -> redis::RedisResult<Option<String>> {
    let v: Option<String> = conn.get(key).await?;
    Ok(v)
}

pub async fn set_string(conn: &mut MultiplexedConnection, key: &str, value: &str, ttl_secs: u64) -> redis::RedisResult<()> {
    let _: () = conn.set_ex(key, value, ttl_secs).await?;
    Ok(())
}
