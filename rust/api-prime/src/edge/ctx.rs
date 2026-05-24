//! Shared edge state — the value injected into the axum service.
//!
//! Distinct from [`crate::state::AppState`] (the read-API handler state)
//! because the edge pipeline has its own collaborators (route table, L1
//! cache, Lambda client). Implement mode threads the existing `AppState`
//! through to the legacy handlers.

use std::sync::Arc;

use crate::edge::cache::EdgeCache;
use crate::edge::edge_attest::EdgeAttestSigner;
use crate::edge::proxy::LambdaProxy;
use crate::edge::ratelimit::RateLimiter;
use crate::edge::route_table::RouteTable;
use crate::state::AppState;

/// Per-request edge context shared across the pipeline. Cheaply `Clone`.
#[derive(Clone)]
pub struct EdgeContext {
    pub routes: Arc<RouteTable>,
    pub cache: Arc<EdgeCache>,
    pub ratelimit: Arc<RateLimiter>,
    pub proxy: Arc<LambdaProxy>,
    pub attest: Arc<EdgeAttestSigner>,
    pub app: AppState,
    /// True iff `IS_LOCAL=true` — turns on debug response headers
    /// unconditionally. When false, debug headers only emit on the
    /// `X-Smoo-Cache-Debug: 1` request header.
    pub debug_default_on: bool,
}
