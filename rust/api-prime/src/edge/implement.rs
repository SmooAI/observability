//! Implement mode — dispatch into in-process Rust handlers.
//!
//! The dispatcher reads `route.implement.rustHandler` and looks it up in
//! a static `phf` map. Each handler is a thin adapter that converts the
//! generic [`InboundRequest`](crate::edge::proxy::InboundRequest) into
//! whatever axum extractors the existing handler needs, then renders
//! the response back into a [`CachedResponse`](crate::edge::cache::CachedResponse).
//!
//! The existing handlers in [`crate::handlers`] are unchanged — this
//! module is the dispatch indirection only. The route table tells us
//! which handler to call; the handler itself is the same code that
//! served the request before the edge pipeline existed.

use std::collections::HashMap;

use crate::edge::cache::{now_secs, CachedResponse};
use crate::edge::proxy::InboundRequest;
use crate::error::AppError;
use crate::state::AppState;

/// Compile-time map of `rust_handler` name → adapter fn. New handlers
/// register here. Keep the entries sorted alphabetically for diff hygiene.
pub static HANDLERS: phf::Map<&'static str, HandlerFn> = phf::phf_map! {
    "health_liveness" => handler_health_liveness as HandlerFn,
    "health_readiness" => handler_health_readiness as HandlerFn,
};

/// Adapter signature — all in-process handlers normalize to this shape.
/// Returning a `CachedResponse` lets cache-mode call this adapter too
/// (though in practice implement-mode routes aren't cached at the edge —
/// the handler is local so there's nothing to amortize).
pub type HandlerFn = for<'a> fn(
    &'a AppState,
    &'a InboundRequest,
    &'a HashMap<String, String>,
) -> futures::future::BoxFuture<'a, Result<CachedResponse, AppError>>;

/// Look up + invoke the named handler. Returns 500 if the name isn't
/// registered (controller is responsible for catching that at manifest
/// validation time; this is the safety net).
pub async fn dispatch(
    name: &str,
    state: &AppState,
    req: &InboundRequest,
    path_params: &HashMap<String, String>,
) -> Result<CachedResponse, AppError> {
    let handler = HANDLERS
        .get(name)
        .ok_or_else(|| AppError::Internal(format!("no Rust handler registered for {name}")))?;
    handler(state, req, path_params).await
}

// ---------- handler adapters ----------

fn handler_health_liveness<'a>(
    _state: &'a AppState,
    _req: &'a InboundRequest,
    _params: &'a HashMap<String, String>,
) -> futures::future::BoxFuture<'a, Result<CachedResponse, AppError>> {
    Box::pin(async move { Ok(json_ok(serde_json::json!({"status": "ok"}))) })
}

fn handler_health_readiness<'a>(
    state: &'a AppState,
    _req: &'a InboundRequest,
    _params: &'a HashMap<String, String>,
) -> futures::future::BoxFuture<'a, Result<CachedResponse, AppError>> {
    Box::pin(async move {
        let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
        let redis_ok = match crate::cache::get_connection(&state.redis).await {
            Ok(mut conn) => {
                let pinged: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
                pinged.is_ok()
            }
            Err(_) => false,
        };
        let status = if db_ok && redis_ok { 200 } else { 503 };
        Ok(CachedResponse {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: serde_json::json!({ "db": db_ok, "redis": redis_ok }).to_string(),
            is_base64_encoded: false,
            cached_at: now_secs(),
            ttl_at: now_secs(),
            swr_at: now_secs(),
        })
    })
}

fn json_ok(value: serde_json::Value) -> CachedResponse {
    let now = now_secs();
    CachedResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: value.to_string(),
        is_base64_encoded: false,
        cached_at: now,
        ttl_at: now,
        swr_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_registry_contains_known_entries() {
        assert!(HANDLERS.contains_key("health_liveness"));
        assert!(HANDLERS.contains_key("health_readiness"));
        assert!(!HANDLERS.contains_key("does_not_exist"));
    }
}
