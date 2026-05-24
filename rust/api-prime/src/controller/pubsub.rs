//! Valkey pub/sub helpers for the controller. The controller is the only
//! writer on these channels — see ADR-017 §"Valkey keyspace".
//!
//! Channel names are pinned constants here; do NOT format them inline at
//! callsites — keeping them in one place lets us grep + the data plane
//! subscribes by exactly these strings.

use anyhow::{Context, Result};
use chrono::Utc;
use redis::aio::MultiplexedConnection;
use serde_json::json;

/// Channel: notifies the data plane to drop its in-proc route map and
/// re-read from `apr:route:*`.
pub const CHANNEL_CONFIG_BUMP: &str = "apr:config-bump";

/// Channel: notifies the data plane to drop L1 cache entries matching the
/// supplied tags.
pub const CHANNEL_INVALIDATE: &str = "apr:invalidate";

/// Publish a config-bump. Payload is the current ISO-8601 timestamp; the
/// data plane only cares that the message arrived (re-reads from Valkey),
/// but the timestamp helps with debugging.
pub async fn publish_config_bump(conn: &mut MultiplexedConnection) -> Result<()> {
    let payload = Utc::now().to_rfc3339();
    let _: i64 = redis::cmd("PUBLISH")
        .arg(CHANNEL_CONFIG_BUMP)
        .arg(&payload)
        .query_async(conn)
        .await
        .with_context(|| format!("PUBLISH {} failed", CHANNEL_CONFIG_BUMP))?;
    tracing::info!(channel = CHANNEL_CONFIG_BUMP, payload = %payload, "published config-bump");
    Ok(())
}

/// Publish an invalidation event for a set of tags. Payload is JSON:
/// `{"tags": ["org:xyz", "user:abc"], "at": "<iso>"}`.
pub async fn publish_invalidation(
    conn: &mut MultiplexedConnection,
    tags: &[String],
) -> Result<()> {
    let payload = json!({
        "tags": tags,
        "at": Utc::now().to_rfc3339(),
    })
    .to_string();
    let _: i64 = redis::cmd("PUBLISH")
        .arg(CHANNEL_INVALIDATE)
        .arg(&payload)
        .query_async(conn)
        .await
        .with_context(|| format!("PUBLISH {} failed", CHANNEL_INVALIDATE))?;
    tracing::info!(channel = CHANNEL_INVALIDATE, tag_count = tags.len(), "published invalidation");
    Ok(())
}
