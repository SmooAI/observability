//! Postgres pool initialization.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub async fn init_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// Build a lazily-connecting pool. Useful in tests and for environments
/// where the DB isn't reachable at boot but the binary should still start.
pub fn lazy_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new().max_connections(10).connect_lazy(database_url)?;
    Ok(pool)
}
