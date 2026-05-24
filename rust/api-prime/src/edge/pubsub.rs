//! Pubsub subscriber for control-plane → data-plane notifications.
//!
//! Subscribes to two Valkey channels:
//!
//! - `apr:config-bump` — drop L1 + re-read route table.
//! - `apr:invalidate`  — JSON `{ "tags": [...] }`, drop matching L1 entries.
//!
//! Resilient to reconnects (exponential backoff capped at 30s).
//! At-least-once delivery — receiving twice is safe; both ops are
//! idempotent drops.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde::Deserialize;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::edge::cache::EdgeCache;
use crate::edge::route_table::RouteTable;

pub const CHANNEL_CONFIG_BUMP: &str = "apr:config-bump";
pub const CHANNEL_INVALIDATE: &str = "apr:invalidate";

#[derive(Debug, Deserialize)]
struct InvalidatePayload {
    tags: Vec<String>,
}

pub fn spawn(client: redis::Client, routes: Arc<RouteTable>, cache: Arc<EdgeCache>) {
    tokio::spawn(async move {
        run(client, routes, cache).await;
    });
}

async fn run(client: redis::Client, routes: Arc<RouteTable>, cache: Arc<EdgeCache>) {
    let mut backoff_secs: u64 = 1;
    loop {
        match subscribe_once(&client, &routes, &cache).await {
            Ok(()) => {
                backoff_secs = 1;
            }
            Err(e) => {
                error!(error = %e, "edge pubsub subscriber dropped, reconnecting");
                sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(30);
            }
        }
    }
}

async fn subscribe_once(
    client: &redis::Client,
    routes: &Arc<RouteTable>,
    cache: &Arc<EdgeCache>,
) -> Result<(), redis::RedisError> {
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(&[CHANNEL_CONFIG_BUMP, CHANNEL_INVALIDATE]).await?;
    info!(channels = ?[CHANNEL_CONFIG_BUMP, CHANNEL_INVALIDATE], "edge pubsub subscribed");

    let mut stream = pubsub.on_message();
    while let Some(msg) = stream.next().await {
        let channel = msg.get_channel_name().to_string();
        let payload = msg.get_payload::<String>().unwrap_or_default();
        handle(channel.as_str(), payload, routes, cache).await;
    }
    Ok(())
}

async fn handle(channel: &str, payload: String, routes: &Arc<RouteTable>, cache: &Arc<EdgeCache>) {
    match channel {
        CHANNEL_CONFIG_BUMP => {
            info!("received config-bump, refreshing route table + clearing L1");
            cache.clear_l1().await;
            if let Err(e) = routes.refresh().await {
                error!(error = %e, "route table refresh failed");
            }
        }
        CHANNEL_INVALIDATE => match serde_json::from_str::<InvalidatePayload>(&payload) {
            Ok(p) => {
                debug!(tags = ?p.tags, "received invalidate");
                cache.invalidate_tags(&p.tags).await;
            }
            Err(e) => warn!(error = %e, payload = %payload, "malformed invalidate payload"),
        },
        other => debug!(channel = %other, "ignoring message on unexpected channel"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidate_payload_parses() {
        let p: InvalidatePayload = serde_json::from_str(r#"{"tags":["a","b"]}"#).unwrap();
        assert_eq!(p.tags, vec!["a".to_string(), "b".to_string()]);
    }
}
