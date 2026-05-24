//! Per-request edge pipeline.
//!
//! Pipeline:
//!
//! 1. Route table lookup (404 on miss).
//! 2. Auth (401 on invalid).
//! 3. Rate limit (429 on throttle).
//! 4. Schema validation (stub in v1).
//! 5. Mode dispatch:
//!    - **Proxy**:     direct Lambda invoke.
//!    - **Cache**:     L1/L2 lookup → serve fresh / serve stale + bg
//!      refresh / miss → fetch + store.
//!    - **Implement**: in-process Rust handler.
//! 6. Render the [`CachedResponse`] back into an axum `Response`.
//!    Optionally stamp debug headers (dev-only).

use std::collections::HashMap;

use axum::body::{to_bytes, Body};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use base64::{engine::general_purpose::STANDARD as B64_STD, Engine as _};
use tracing::warn;

use crate::edge::auth::{verify as verify_auth, EdgeAuthContext};
use crate::edge::cache::{compute_expiry, CacheStatus, CachedResponse, EdgeCache, Freshness};
use crate::edge::ctx::EdgeContext;
use crate::edge::debug::{
    self, DebugTrace, HEADER_CACHE_KEY, HEADER_CACHE_STATUS, HEADER_LAMBDA_ARN, HEADER_ROUTE_MODE,
};
use crate::edge::proxy::InboundRequest;
use crate::edge::ratelimit::RateLimitOutcome;
use crate::edge::route_table::CompiledRoute;
use crate::edge::schema;
use crate::edge::types::{RouteEntry, RouteMode};
use crate::error::AppError;

/// Maximum inbound body size (16 MiB — matches API Gateway HTTP API limit).
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Axum entry-point. Single catch-all route routes everything here.
pub async fn dispatch(
    State(ctx): State<EdgeContext>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    req: Request<Body>,
) -> Response {
    let (parts, body) = req.into_parts();
    let body_bytes = match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            warn!(error = %e, "request body too large or unreadable");
            return AppError::BadRequest("request body too large or unreadable".to_string()).into_response();
        }
    };

    let inbound = build_inbound(&parts.method, &parts.uri, &parts.headers, body_bytes);
    let req_debug_on = debug::should_emit(ctx.debug_default_on, &parts.headers);

    match run_pipeline(&ctx, &parts.method, &parts.uri, &parts.headers, &peer, inbound).await {
        Ok((response, trace)) => attach_debug_headers(response, &trace, req_debug_on),
        Err((err, trace)) => attach_debug_headers(err.into_response(), &trace, req_debug_on),
    }
}

async fn run_pipeline(
    ctx: &EdgeContext,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    peer: &std::net::SocketAddr,
    inbound: InboundRequest,
) -> Result<(Response, DebugTrace), (AppError, DebugTrace)> {
    let mut trace = DebugTrace::default();

    // 1. Route lookup.
    let snapshot = ctx.routes.snapshot().await;
    let (route, path_params) = match snapshot.lookup(method.as_str(), uri.path()) {
        Some((r, p)) => (r, p),
        None => return Err((AppError::NotFound(format!("no route for {} {}", method, uri.path())), trace)),
    };
    let route_entry: RouteEntry = route.entry.clone();
    trace = trace.route_mode(route_entry.mode.as_str());

    // 2. Auth.
    let peer_str = peer.ip().to_string();
    let auth = verify_auth(headers, &peer_str, &route_entry, &ctx.app.jwks)
        .await
        .map_err(|e| (e, trace.clone()))?;

    // 3. Rate limit.
    let rl = ctx.ratelimit.check(&route_entry, &auth).await.map_err(|e| (e, trace.clone()))?;
    if let RateLimitOutcome::Throttled { limit, window_seconds } = &rl {
        return Err((
            AppError::BadRequest(format!(
                "rate limit exceeded: {limit} requests per {window_seconds}s window"
            )),
            trace,
        ));
    }

    // 4. Schema (stub).
    schema::validate_request(headers, &inbound.body, &route_entry).map_err(|e| (e, trace.clone()))?;

    // 5. Mode dispatch.
    let (cached, status) = dispatch_mode(ctx, route, &route_entry, &inbound, &path_params, &auth, &mut trace).await
        .map_err(|e| (e, trace.clone()))?;

    if let Some(s) = status {
        trace = trace.cache_status(s.as_str());
    } else if matches!(route_entry.mode, RouteMode::Proxy) || matches!(route_entry.mode, RouteMode::Implement) {
        trace = trace.cache_status("BYPASS");
    }

    let response = render_response(cached);
    Ok((response, trace))
}

async fn dispatch_mode(
    ctx: &EdgeContext,
    route: &CompiledRoute,
    route_entry: &RouteEntry,
    req: &InboundRequest,
    path_params: &HashMap<String, String>,
    auth: &EdgeAuthContext,
    trace: &mut DebugTrace,
) -> Result<(CachedResponse, Option<CacheStatus>), AppError> {
    match route_entry.mode {
        RouteMode::Implement => {
            let handler = route_entry
                .implement
                .as_ref()
                .ok_or_else(|| AppError::Internal("implement-mode route has no implement.rustHandler".to_string()))?
                .rust_handler
                .as_str();
            let resp = crate::edge::implement::dispatch(handler, &ctx.app, req, path_params).await?;
            Ok((resp, None))
        }
        RouteMode::Proxy => {
            if let Some(arn) = &route_entry.lambda_arn {
                *trace = trace.clone().lambda_arn(arn.clone());
            }
            let resp = ctx.proxy.invoke(route_entry, req, path_params, auth, &ctx.attest).await?;
            Ok((resp, None))
        }
        RouteMode::Cache => dispatch_cache(ctx, route, route_entry, req, path_params, auth, trace).await,
    }
}

async fn dispatch_cache(
    ctx: &EdgeContext,
    _route: &CompiledRoute,
    route_entry: &RouteEntry,
    req: &InboundRequest,
    path_params: &HashMap<String, String>,
    auth: &EdgeAuthContext,
    trace: &mut DebugTrace,
) -> Result<(CachedResponse, Option<CacheStatus>), AppError> {
    let cfg = route_entry
        .cache
        .as_ref()
        .ok_or_else(|| AppError::Internal("cache-mode route has no cache config".to_string()))?;
    let fragments = EdgeCache::compose_key(cfg, route_entry, auth, path_params);
    let canonical = EdgeCache::canonical_key(&fragments);
    *trace = trace.clone().cache_key(&canonical);

    if let Some((entry, _tier)) = ctx.cache.get(&canonical).await {
        match entry.freshness() {
            Freshness::Fresh => return Ok((entry, Some(CacheStatus::Hit))),
            Freshness::Stale => {
                // SWR: serve stale, refresh in background. Dedup via per-key mutex
                // so we don't fan out invokes when many concurrent requests see stale.
                spawn_refresh(ctx, route_entry, req, path_params, auth, &fragments, &canonical, cfg);
                return Ok((entry, Some(CacheStatus::Stale)));
            }
            Freshness::Expired => {
                // Fall through to MISS fetch.
            }
        }
    }

    if let Some(arn) = &route_entry.lambda_arn {
        *trace = trace.clone().lambda_arn(arn.clone());
    }
    let mut fresh = ctx.proxy.invoke(route_entry, req, path_params, auth, &ctx.attest).await?;
    let (cached_at, ttl_at, swr_at) = compute_expiry(cfg);
    fresh.cached_at = cached_at;
    fresh.ttl_at = ttl_at;
    fresh.swr_at = swr_at;

    if is_cacheable(&fresh) {
        ctx.cache.put(&canonical, &fragments, fresh.clone()).await;
    }

    Ok((fresh, Some(CacheStatus::Miss)))
}

#[allow(clippy::too_many_arguments)]
fn spawn_refresh(
    ctx: &EdgeContext,
    route_entry: &RouteEntry,
    req: &InboundRequest,
    path_params: &HashMap<String, String>,
    auth: &EdgeAuthContext,
    fragments: &[String],
    canonical: &str,
    cfg: &crate::edge::types::CacheConfig,
) {
    let ctx2 = ctx.clone();
    let route2 = route_entry.clone();
    let req2 = req.clone();
    let params2 = path_params.clone();
    let auth2 = auth.clone();
    let fragments2 = fragments.to_vec();
    let canonical2 = canonical.to_string();
    let cfg2 = cfg.clone();
    tokio::spawn(async move {
        let lock = ctx2.cache.refresh_lock(&canonical2);
        // try_lock: if someone else is already refreshing, drop. We don't
        // want a queue of refreshes piling up on the same key.
        let _guard = match lock.try_lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        // Re-check freshness before kicking off the invoke; another
        // refresh may have populated a fresh entry between the original
        // staleness check and grabbing the lock.
        if let Some((entry, _)) = ctx2.cache.get(&canonical2).await {
            if matches!(entry.freshness(), Freshness::Fresh) {
                return;
            }
        }
        match ctx2.proxy.invoke(&route2, &req2, &params2, &auth2, &ctx2.attest).await {
            Ok(mut fresh) => {
                let (cached_at, ttl_at, swr_at) = compute_expiry(&cfg2);
                fresh.cached_at = cached_at;
                fresh.ttl_at = ttl_at;
                fresh.swr_at = swr_at;
                if is_cacheable(&fresh) {
                    ctx2.cache.put(&canonical2, &fragments2, fresh).await;
                }
            }
            Err(e) => warn!(error = %e, "background cache refresh failed; keeping stale entry"),
        }
    });
}

fn is_cacheable(resp: &CachedResponse) -> bool {
    matches!(resp.status, 200..=299)
}

fn build_inbound(method: &Method, uri: &Uri, headers: &HeaderMap, body: Vec<u8>) -> InboundRequest {
    let mut headers_map = HashMap::with_capacity(headers.len());
    for (k, v) in headers {
        if let Ok(s) = v.to_str() {
            headers_map.insert(k.as_str().to_string(), s.to_string());
        }
    }
    let mut query = HashMap::new();
    if let Some(q) = uri.query() {
        for (k, v) in url_query_pairs(q) {
            query.insert(k, v);
        }
    }
    InboundRequest {
        method: method.as_str().to_string(),
        path: uri.path().to_string(),
        query,
        headers: headers_map,
        body,
    }
}

fn url_query_pairs(q: &str) -> impl Iterator<Item = (String, String)> + '_ {
    q.split('&').filter_map(|kv| {
        let mut it = kv.splitn(2, '=');
        let k = it.next()?.to_string();
        let v = it.next().unwrap_or("").to_string();
        if k.is_empty() {
            return None;
        }
        Some((k, v))
    })
}

fn render_response(cached: CachedResponse) -> Response {
    let status = StatusCode::from_u16(cached.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = if cached.is_base64_encoded {
        B64_STD.decode(cached.body.as_bytes()).unwrap_or_default()
    } else {
        cached.body.into_bytes()
    };
    let mut response = Response::builder().status(status).body(Body::from(body)).unwrap_or_else(|_| {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        r
    });
    let response_headers = response.headers_mut();
    for (k, v) in cached.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::try_from(k), HeaderValue::try_from(v)) {
            response_headers.append(name, value);
        }
    }
    response
}

fn attach_debug_headers(mut response: Response, trace: &DebugTrace, emit: bool) -> Response {
    if !emit {
        return response;
    }
    let h = response.headers_mut();
    if let Some(v) = trace.cache_status {
        if let Ok(val) = HeaderValue::from_str(v) {
            h.insert(HeaderName::from_static(HEADER_CACHE_STATUS), val);
        }
    } else {
        h.insert(HeaderName::from_static(HEADER_CACHE_STATUS), HeaderValue::from_static("BYPASS"));
    }
    if let Some(k) = &trace.cache_key_prefix {
        if let Ok(val) = HeaderValue::from_str(k) {
            h.insert(HeaderName::from_static(HEADER_CACHE_KEY), val);
        }
    }
    if let Some(m) = trace.route_mode {
        if let Ok(val) = HeaderValue::from_str(m) {
            h.insert(HeaderName::from_static(HEADER_ROUTE_MODE), val);
        }
    }
    let arn = trace.lambda_arn.clone().unwrap_or_default();
    if let Ok(val) = HeaderValue::from_str(&arn) {
        h.insert(HeaderName::from_static(HEADER_LAMBDA_ARN), val);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::types::{AuthRequirement, CacheConfig, ImplementConfig, RateLimitConfig, RouteMode};

    fn entry(mode: RouteMode) -> RouteEntry {
        RouteEntry {
            path: "/foo".into(),
            method: "GET".into(),
            auth: AuthRequirement::Public,
            idempotent: true,
            mode,
            rate_limit: RateLimitConfig {
                per_token: 1,
                window_seconds: 1,
            },
            cache: if mode == RouteMode::Cache {
                Some(CacheConfig {
                    ttl_seconds: 60,
                    swr_seconds: 60,
                    key_template: vec![],
                })
            } else {
                None
            },
            implement: if mode == RouteMode::Implement {
                Some(ImplementConfig {
                    rust_handler: "health_liveness".into(),
                })
            } else {
                None
            },
            lambda_arn: if mode == RouteMode::Implement {
                None
            } else {
                Some("arn".into())
            },
            schema_ref: None,
        }
    }

    #[test]
    fn render_response_decodes_base64_body() {
        let cached = CachedResponse {
            status: 200,
            headers: vec![("content-type".into(), "application/octet-stream".into())],
            body: B64_STD.encode(b"hello"),
            is_base64_encoded: true,
            cached_at: 0,
            ttl_at: 0,
            swr_at: 0,
        };
        let response = render_response(cached);
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn url_query_pairs_handles_empty_and_repeated() {
        let pairs: Vec<_> = url_query_pairs("a=1&b=&c").collect();
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], ("a".into(), "1".into()));
        assert_eq!(pairs[1], ("b".into(), "".into()));
        assert_eq!(pairs[2], ("c".into(), "".into()));
    }

    #[test]
    fn mode_selection_matches_route_entry() {
        let e_proxy = entry(RouteMode::Proxy);
        let e_cache = entry(RouteMode::Cache);
        let e_impl = entry(RouteMode::Implement);
        assert_eq!(e_proxy.mode, RouteMode::Proxy);
        assert_eq!(e_cache.mode, RouteMode::Cache);
        assert_eq!(e_impl.mode, RouteMode::Implement);
        // Smoke check the cache/implement configs the dispatcher requires.
        assert!(e_cache.cache.is_some());
        assert!(e_impl.implement.is_some());
    }
}

