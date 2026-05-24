//! Dev-only response headers.
//!
//! Enabled when EITHER:
//! - `IS_LOCAL=true` env (decided once at boot, lives in `EdgeContext`), OR
//! - request header `X-Smoo-Cache-Debug: 1`
//!
//! When neither is true the dispatcher MUST NOT emit these headers —
//! they leak cache-key fingerprints and Lambda ARNs to the public.
//!
//! The four headers we emit:
//! - `X-Smoo-Cache-Status`  : `HIT | MISS | STALE | BYPASS`
//! - `X-Smoo-Cache-Key`     : first 8 hex chars of the SHA-256 cache key
//! - `X-Smoo-Route-Mode`    : `proxy | cache | implement`
//! - `X-Smoo-Lambda-Arn`    : ARN actually invoked (empty for implement / cache-HIT)

use axum::http::HeaderMap;

pub const HEADER_DEBUG_OPT_IN: &str = "x-smoo-cache-debug";
pub const HEADER_CACHE_STATUS: &str = "x-smoo-cache-status";
pub const HEADER_CACHE_KEY: &str = "x-smoo-cache-key";
pub const HEADER_ROUTE_MODE: &str = "x-smoo-route-mode";
pub const HEADER_LAMBDA_ARN: &str = "x-smoo-lambda-arn";

#[derive(Debug, Clone, Default)]
pub struct DebugTrace {
    pub cache_status: Option<&'static str>,
    pub cache_key_prefix: Option<String>,
    pub route_mode: Option<&'static str>,
    pub lambda_arn: Option<String>,
}

impl DebugTrace {
    pub fn cache_status(mut self, s: &'static str) -> Self {
        self.cache_status = Some(s);
        self
    }
    pub fn cache_key(mut self, full_key: &str) -> Self {
        self.cache_key_prefix = Some(full_key.chars().take(8).collect());
        self
    }
    pub fn route_mode(mut self, s: &'static str) -> Self {
        self.route_mode = Some(s);
        self
    }
    pub fn lambda_arn(mut self, arn: impl Into<String>) -> Self {
        self.lambda_arn = Some(arn.into());
        self
    }
}

/// Returns true iff debug headers should be emitted for this request.
pub fn should_emit(default_on: bool, req_headers: &HeaderMap) -> bool {
    if default_on {
        return true;
    }
    matches!(
        req_headers.get(HEADER_DEBUG_OPT_IN).and_then(|v| v.to_str().ok()),
        Some("1") | Some("true")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_default_emits() {
        let h = HeaderMap::new();
        assert!(should_emit(true, &h));
    }

    #[test]
    fn header_opt_in_emits() {
        let mut h = HeaderMap::new();
        h.insert(HEADER_DEBUG_OPT_IN, "1".parse().unwrap());
        assert!(should_emit(false, &h));
    }

    #[test]
    fn prod_default_silent() {
        let h = HeaderMap::new();
        assert!(!should_emit(false, &h));
    }

    #[test]
    fn debug_trace_truncates_key() {
        let t = DebugTrace::default().cache_key("0123456789abcdef0123456789abcdef");
        assert_eq!(t.cache_key_prefix.unwrap(), "01234567");
    }
}
