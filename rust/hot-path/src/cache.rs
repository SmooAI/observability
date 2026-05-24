//! Redis client helpers. Used for read-through caching of `/v1/profile`
//! (key: `profile:{user_id}`, TTL 5min) plus shared cache hits with the
//! existing TS backend that uses the same Redis instance.

use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;

pub const PROFILE_TTL_SECS: u64 = 300;

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

pub async fn get_string(conn: &mut MultiplexedConnection, key: &str) -> redis::RedisResult<Option<String>> {
    let v: Option<String> = conn.get(key).await?;
    Ok(v)
}

pub async fn set_string(conn: &mut MultiplexedConnection, key: &str, value: &str, ttl_secs: u64) -> redis::RedisResult<()> {
    let _: () = conn.set_ex(key, value, ttl_secs).await?;
    Ok(())
}
