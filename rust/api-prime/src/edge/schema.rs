//! Schema validation — request and response.
//!
//! **v1 STUB.** Real implementation lands in SMOODEV-1277. We expose the
//! call sites now so the dispatcher's pipeline is fully wired; the
//! validator just logs the `schemaRef` and accepts. Egress validation
//! will run warn-only for 30 days post-launch (ADR-017 §"Schema validation").

use axum::http::HeaderMap;
use tracing::trace;

use crate::edge::types::RouteEntry;
use crate::error::AppError;

pub fn validate_request(_headers: &HeaderMap, _body: &[u8], route: &RouteEntry) -> Result<(), AppError> {
    if let Some(schema) = &route.schema_ref {
        trace!(schema = %schema, "TODO schema validation (SMOODEV-1277)");
    }
    Ok(())
}

pub fn validate_response(_body: &[u8], route: &RouteEntry) -> Result<(), AppError> {
    if let Some(schema) = &route.schema_ref {
        trace!(schema = %schema, "TODO egress schema validation (SMOODEV-1277)");
    }
    Ok(())
}
