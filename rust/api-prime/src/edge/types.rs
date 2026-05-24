//! Route table entry types — Rust mirror of the controller's TS `RouteEntry`
//! (see `packages/api-prime-manifest/` in the smooai monorepo and
//! ADR-017 §"Route manifest").
//!
//! The controller writes JSON-encoded entries to `apr:route:<METHOD>:<path>`.
//! The data plane only reads.

use serde::{Deserialize, Serialize};

/// Per-route auth requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthRequirement {
    /// Bearer JWT issued by Supabase Auth.
    User,
    /// Machine-to-machine token (SST Auth client_credentials grant).
    M2m,
    /// No auth required.
    Public,
}

/// Per-route backend mode — what the dispatcher actually does on a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    /// Forward to the per-route Lambda via direct InvokeFunction.
    Proxy,
    /// Same as Proxy on miss, but with L1+L2 cache + SWR semantics.
    Cache,
    /// Dispatch into an in-process Rust handler (no Lambda involved).
    Implement,
}

impl RouteMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RouteMode::Proxy => "proxy",
            RouteMode::Cache => "cache",
            RouteMode::Implement => "implement",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitConfig {
    /// Maximum requests per window per auth subject (or per IP for public routes).
    pub per_token: u32,
    /// Sliding-window length, in seconds.
    pub window_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheConfig {
    /// Hard TTL — entries strictly older than this are MISS.
    pub ttl_seconds: u32,
    /// Stale-while-revalidate window.
    pub swr_seconds: u32,
    /// Composed key fragments. Each is a template like `"user:{auth.sub}"`
    /// or `"org:{:org_id}"`.
    pub key_template: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImplementConfig {
    /// Name of the compiled Rust handler. Looked up in
    /// [`crate::edge::implement::HANDLERS`].
    pub rust_handler: String,
}

/// Single entry in the route table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEntry {
    /// Path with `:param` placeholders.
    pub path: String,
    pub method: String,
    pub auth: AuthRequirement,
    /// Hint for retry safety. Not enforced by the edge.
    pub idempotent: bool,
    pub mode: RouteMode,
    pub rate_limit: RateLimitConfig,
    #[serde(default)]
    pub cache: Option<CacheConfig>,
    #[serde(default)]
    pub implement: Option<ImplementConfig>,
    /// Lambda ARN resolved by the controller. Required for proxy + cache.
    #[serde(default)]
    pub lambda_arn: Option<String>,
    /// OpenAPI registry slug (stubbed validation in v1).
    #[serde(default)]
    pub schema_ref: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_controller_payload() {
        let raw = r#"{
            "path": "/organizations/:org_id/products",
            "method": "GET",
            "auth": "user",
            "idempotent": true,
            "mode": "cache",
            "rateLimit": {"perToken": 60, "windowSeconds": 60},
            "cache": {"ttlSeconds": 120, "swrSeconds": 60, "keyTemplate": ["user:{auth.sub}", "org:{:org_id}"]},
            "lambdaArn": "arn:aws:lambda:us-east-2:1:function:smooai-production-foo",
            "schemaRef": "OrganizationProducts"
        }"#;
        let entry: RouteEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.method, "GET");
        assert_eq!(entry.mode, RouteMode::Cache);
        assert_eq!(entry.auth, AuthRequirement::User);
        assert_eq!(entry.cache.as_ref().unwrap().ttl_seconds, 120);
        assert_eq!(entry.rate_limit.per_token, 60);
    }

    #[test]
    fn implement_mode_round_trip() {
        let raw = r#"{
            "path": "/health/liveness", "method": "GET", "auth": "public", "idempotent": true,
            "mode": "implement", "rateLimit": {"perToken": 1000, "windowSeconds": 60},
            "implement": {"rustHandler": "health_liveness"}
        }"#;
        let entry: RouteEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.mode, RouteMode::Implement);
        assert_eq!(entry.implement.as_ref().unwrap().rust_handler, "health_liveness");
    }
}
