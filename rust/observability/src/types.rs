//! Public event types — serde structs that mirror the `@smooai/observability`
//! TypeScript wire format exactly.
//!
//! These mirror the Sentry "event envelope" shape closely enough that the
//! backend can fingerprint and store them without inventing a parallel schema,
//! while remaining first-class for Smoo (no Sentry dependency, no Sentry DSN).
//!
//! The JSON field names match the TS SDK (`types.ts`) byte-for-byte: camelCase
//! keys, `Option` fields are skipped when `None` so the payload is identical to
//! what the TS transport POSTs to `/webhooks/observability/{org_id}/{token}`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Severity. Most captured exceptions are [`Level::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Fatal,
    Error,
    Warning,
    #[default]
    Info,
    Debug,
}

/// SDK self-identification runtime. Rust services report `node` so they share
/// the TS Node ingest path on the backend (the backend only distinguishes
/// `browser` vs `node`; there is no separate `rust` bucket).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Browser,
    Node,
}

/// User context, if known. Mirrors `ObservabilityEvent['user']` in the TS SDK.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl UserContext {
    /// True if no field is populated. Used so an all-empty user is omitted from
    /// the event rather than serialized as `{}`.
    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.org_id.is_none() && self.session_id.is_none()
    }
}

/// A single stack frame, innermost (most recent) first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    /// Filename or module identifier.
    pub module: String,
    /// Function name from the stack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// Line number in the original source if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineno: Option<u32>,
    /// Column number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colno: Option<u32>,
    /// True if the frame is application code (not vendored / sdk-internal).
    #[serde(skip_serializing_if = "Option::is_none", rename = "inApp")]
    pub in_app: Option<bool>,
}

/// Wrapper matching the TS `{ frames: StackFrame[] }` shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackTrace {
    pub frames: Vec<StackFrame>,
}

/// Exception chain entry (innermost first). Mirrors `ExceptionInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionInfo {
    /// e.g. `TypeError`, `std::io::Error`, the Rust type name.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Exception message (`Display` / `Debug` of the error).
    pub value: String,
    /// Stack frames, innermost (most recent) first.
    pub stacktrace: StackTrace,
    /// Linked cause (error-source chain), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<Box<ExceptionInfo>>,
}

/// Breadcrumb leading up to an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Breadcrumb {
    /// ms since epoch.
    pub timestamp: i64,
    /// Free-form category — `fetch`, `db`, `navigation`, `custom`, …
    pub category: String,
    /// `info` for most, `warning` / `error` for failures.
    pub level: Level,
    /// Short human-readable summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Free-form structured data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Request / invocation context. Mirrors `RequestInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Selected headers (PII-scrubbed by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_string: Option<String>,
}

impl RequestInfo {
    pub fn is_empty(&self) -> bool {
        self.url.is_none()
            && self.method.is_none()
            && self.headers.is_none()
            && self.query_string.is_none()
    }
}

/// SDK self-identification block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkInfo {
    pub name: String,
    pub version: String,
    pub runtime: Runtime,
}

/// The canonical event envelope. Field names + skip-when-none behavior match
/// `ObservabilityEvent` in the TS SDK so the backend parses Rust + TS events
/// with one schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilityEvent {
    /// Client-assigned event id (UUID v4).
    pub event_id: String,
    /// When the event occurred, ms since epoch.
    pub timestamp: i64,
    /// Severity.
    pub level: Level,
    /// Optional one-line message — for `capture_message`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Exception chain (innermost first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<Vec<ExceptionInfo>>,
    /// Breadcrumb buffer leading up to this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breadcrumbs: Option<Vec<Breadcrumb>>,
    /// User context, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserContext>,
    /// Request / invocation context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestInfo>,
    /// Free-form tags for filtering in the dashboard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<BTreeMap<String, String>>,
    /// Free-form contexts (e.g., os, device, runtime).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contexts: Option<BTreeMap<String, serde_json::Value>>,
    /// Release identifier — git sha, container version, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    /// Deployment environment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// SDK self-identification.
    pub sdk: SdkInfo,
}

/// The transport envelope POSTed to the Smoo ingest endpoint. Discriminated
/// union (`type: 'error'`) matching the TS `IngestPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestPayload {
    #[serde(rename = "type")]
    pub r#type: IngestType,
    pub events: Vec<ObservabilityEvent>,
}

/// Discriminator for [`IngestPayload`]. Only `error` is used today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IngestType {
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Level::Error).unwrap(), "\"error\"");
        assert_eq!(
            serde_json::to_string(&Level::Warning).unwrap(),
            "\"warning\""
        );
    }

    #[test]
    fn runtime_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Runtime::Node).unwrap(), "\"node\"");
    }

    #[test]
    fn event_omits_none_fields_and_uses_camelcase() {
        let event = ObservabilityEvent {
            event_id: "abc".into(),
            timestamp: 123,
            level: Level::Error,
            message: None,
            exception: None,
            breadcrumbs: None,
            user: None,
            request: None,
            tags: None,
            contexts: None,
            release: None,
            environment: None,
            sdk: SdkInfo {
                name: "@smooai/observability".into(),
                version: "0.1.0".into(),
                runtime: Runtime::Node,
            },
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["eventId"], "abc");
        assert_eq!(json["timestamp"], 123);
        assert_eq!(json["level"], "error");
        assert!(json.get("message").is_none(), "None fields must be omitted");
        assert!(json.get("exception").is_none());
        assert_eq!(json["sdk"]["runtime"], "node");
    }

    #[test]
    fn in_app_renders_as_camel_case() {
        let frame = StackFrame {
            module: "src/main.rs".into(),
            function: Some("main".into()),
            lineno: Some(10),
            colno: None,
            in_app: Some(true),
        };
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["inApp"], true);
        assert!(json.get("colno").is_none());
    }

    #[test]
    fn exception_type_field_is_type() {
        let exc = ExceptionInfo {
            r#type: "std::io::Error".into(),
            value: "file not found".into(),
            stacktrace: StackTrace::default(),
            cause: None,
        };
        let json = serde_json::to_value(&exc).unwrap();
        assert_eq!(json["type"], "std::io::Error");
    }

    #[test]
    fn ingest_payload_type_is_error() {
        let payload = IngestPayload {
            r#type: IngestType::Error,
            events: vec![],
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["type"], "error");
        assert!(json["events"].is_array());
    }

    #[test]
    fn request_info_camelcase_query_string() {
        let req = RequestInfo {
            url: Some("/x".into()),
            method: Some("GET".into()),
            headers: None,
            query_string: Some("a=1".into()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["queryString"], "a=1");
    }
}
