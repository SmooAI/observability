//! Load + parse the route manifest JSON emitted by
//! `@smooai/api-prime-manifest` (`pnpm --filter @smooai/api-prime-manifest gen-rust`).
//!
//! The manifest is mounted into the controller pod via k8s ConfigMap at the
//! path given by `MANIFEST_PATH` (default `/etc/api-prime/manifest.json`).
//! S3 support is intentionally not wired here — the ConfigMap mount keeps
//! the manifest in-band with the deploy and avoids the S3 + IAM round trip
//! on every reconcile.

use std::path::Path;

use anyhow::{Context, Result};

use crate::controller::types::RouteEntry;

/// JSON envelope written by the manifest generator. Kept stable so
/// `gen-rust` can add metadata fields (version, generatedAt, etc.) without
/// breaking the controller.
#[derive(Debug, serde::Deserialize)]
struct ManifestEnvelope {
    routes: Vec<RouteEntry>,
}

/// Load + parse the manifest JSON from disk.
pub async fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Vec<RouteEntry>> {
    let path = path.as_ref();
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read manifest at {}", path.display()))?;
    parse_bytes(&bytes)
}

/// Parse manifest JSON bytes. Exposed for tests; production code should
/// prefer [`load_from_file`].
pub fn parse_bytes(bytes: &[u8]) -> Result<Vec<RouteEntry>> {
    // Tolerate both shapes: a bare array `[{...}]` and an envelope
    // `{"routes": [...]}`. The TS generator emits the envelope form today;
    // legacy fixtures use the bare array.
    if let Ok(envelope) = serde_json::from_slice::<ManifestEnvelope>(bytes) {
        return Ok(envelope.routes);
    }
    let routes: Vec<RouteEntry> = serde_json::from_slice(bytes)
        .context("manifest JSON did not match RouteEntry[] or { routes: RouteEntry[] }")?;
    Ok(routes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_envelope_form() {
        let json = br#"{"routes":[{"path":"/v1/profile","method":"GET","auth":"user","idempotent":true,"mode":"implement","rateLimit":{"perToken":100,"windowSeconds":60},"implement":{"rustHandler":"profile"},"schemaRef":"Profile"}]}"#;
        let routes = parse_bytes(json).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].path, "/v1/profile");
    }

    #[test]
    fn parses_bare_array_form() {
        let json = br#"[{"path":"/v1/profile","method":"GET","auth":"user","idempotent":true,"mode":"proxy","rateLimit":{"perToken":100,"windowSeconds":60},"lambdaOutputKey":"ApiRouteProfile","schemaRef":"Profile"}]"#;
        let routes = parse_bytes(json).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].lambda_output_key.as_deref(), Some("ApiRouteProfile"));
    }

    #[test]
    fn rejects_invalid_json() {
        let err = parse_bytes(b"not json").unwrap_err();
        assert!(err.to_string().contains("manifest JSON did not match"));
    }
}
