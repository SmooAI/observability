//! Shared application state injected into axum handlers.

use sqlx::PgPool;

use crate::auth::jwt::JwksCache;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub redis: redis::Client,
    pub jwks: JwksCache,
    pub http: reqwest::Client,
    pub supabase_url: String,
    pub supabase_anon_key: String,
}
