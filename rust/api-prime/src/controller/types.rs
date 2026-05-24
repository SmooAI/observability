//! Types shared by the controller modules. The `RouteEntry` shape mirrors
//! the TS `RouteEntry` exported by `@smooai/api-prime-manifest` — see
//! `packages/api-prime-manifest/src/routes/*.ts` in the smooai repo and
//! ADR-017 §"Route manifest".
//!
//! Field ordering + naming intentionally matches the TS so the JSON the
//! manifest generator emits round-trips cleanly without a separate adapter.

use serde::{Deserialize, Serialize};

/// HTTP method as it appears in the manifest.
///
/// Stored verbatim as an uppercase string in the manifest JSON; we keep it
/// as a `String` rather than an enum so future methods (HEAD, OPTIONS) don't
/// require a controller redeploy.
pub type HttpMethod = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthClass {
    User,
    M2m,
    Public,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    Proxy,
    Cache,
    Implement,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(rename = "perToken")]
    pub per_token: u32,
    #[serde(rename = "windowSeconds")]
    pub window_seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(rename = "ttlSeconds")]
    pub ttl_seconds: u32,
    #[serde(rename = "swrSeconds")]
    pub swr_seconds: u32,
    #[serde(rename = "keyTemplate")]
    pub key_template: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementConfig {
    /// Name of the registered Rust handler. Must appear in the controller's
    /// registered-handlers list or PUT /admin/v1/routes/:id/mode to
    /// `implement` is rejected.
    #[serde(rename = "rustHandler")]
    pub rust_handler: String,
}

/// Source-of-truth route entry as it appears in the manifest JSON.
///
/// Mirrors `RouteEntry` from `@smooai/api-prime-manifest`. `lambda_arn` is
/// optional here because the manifest does not know the ARN — the
/// controller resolves it from SST outputs during reconcile, producing a
/// [`ResolvedRouteEntry`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEntry {
    pub path: String,
    pub method: HttpMethod,
    pub auth: AuthClass,
    pub idempotent: bool,
    pub mode: RouteMode,
    #[serde(rename = "rateLimit")]
    pub rate_limit: RateLimitConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implement: Option<ImplementConfig>,
    /// Key in SST stack outputs that holds the Lambda ARN for this route.
    /// Only meaningful for `proxy`/`cache` modes; `implement` mode ignores
    /// this.
    #[serde(
        rename = "lambdaOutputKey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub lambda_output_key: Option<String>,
    /// OpenAPI registry slug. Opaque to the controller — emitted to the
    /// data plane for schema validation.
    #[serde(rename = "schemaRef")]
    pub schema_ref: String,
}

/// A `RouteEntry` after reconcile has resolved the Lambda ARN from SST
/// outputs. This is the shape stored at `apr:route:<METHOD>:<path>` in
/// Valkey and read by the data plane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRouteEntry {
    #[serde(flatten)]
    pub entry: RouteEntry,
    /// Resolved Lambda ARN; populated for `proxy`/`cache` modes when the
    /// corresponding `lambdaOutputKey` is present in SST outputs. `None`
    /// for `implement` mode or when SST hasn't deployed the route yet.
    #[serde(rename = "lambdaArn", default, skip_serializing_if = "Option::is_none")]
    pub lambda_arn: Option<String>,
}

impl ResolvedRouteEntry {
    /// Stable Valkey key for this route. `METHOD` is uppercased + `path`
    /// is taken verbatim. We do NOT URL-encode here because the manifest
    /// is the source of truth for path normalization.
    pub fn valkey_key(&self) -> String {
        format!(
            "apr:route:{}:{}",
            self.entry.method.to_ascii_uppercase(),
            self.entry.path
        )
    }

    /// Stable identifier used by admin API path params. URL-safe.
    pub fn route_id(&self) -> String {
        format!(
            "{}:{}",
            self.entry.method.to_ascii_uppercase(),
            self.entry.path
        )
    }
}
