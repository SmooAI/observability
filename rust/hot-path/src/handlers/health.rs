//! Liveness + readiness probes for k8s.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use crate::cache;
use crate::state::AppState;

pub async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

pub async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    let redis_ok = match cache::get_connection(&state.redis).await {
        Ok(mut conn) => {
            // Minimal PING via SET on a throwaway key would be intrusive; rely on
            // connection establishment which already exchanges a hello.
            let pinged: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
            pinged.is_ok()
        }
        Err(_) => false,
    };
    let status = if db_ok && redis_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(json!({ "db": db_ok, "redis": redis_ok })))
}
