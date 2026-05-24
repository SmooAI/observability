//! Errors view types + typed `OrgClient` methods.
//!
//! Mirrors `packages/backend/src/routes/observability/errors-query.ts` —
//! list groups, fetch a single group with its recent events, list further
//! events with cursor pagination, and PATCH status mutations.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ApiError, OrgClient};

impl<'a> OrgClient<'a> {
    /// `GET /errors` — first page of error groups filtered by status + env.
    pub async fn list_error_groups(
        &self,
        params: &ErrorListParams,
    ) -> Result<ErrorPage, ApiError> {
        self.get("errors", Some(params)).await
    }

    /// `GET /errors/{group_id}` — group metadata + most recent events.
    pub async fn get_error_group(
        &self,
        group_id: Uuid,
    ) -> Result<ErrorDetail, ApiError> {
        self.get::<ErrorDetail, ()>(&format!("errors/{group_id}"), None).await
    }

    /// `GET /errors/{group_id}/events?cursor=…&limit=…` — paginate events.
    pub async fn list_group_events(
        &self,
        group_id: Uuid,
        params: &PageParams,
    ) -> Result<ErrorEventPage, ApiError> {
        self.get(&format!("errors/{group_id}/events"), Some(params)).await
    }

    /// `PATCH /errors/{group_id}` — mark resolved / muted, reassign.
    pub async fn update_error_group(
        &self,
        group_id: Uuid,
        patch: &ErrorPatch,
    ) -> Result<ErrorGroup, ApiError> {
        self.patch(&format!("errors/{group_id}"), patch).await
    }
}

// ----- Request types ---------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct ErrorListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ErrorStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PageParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ErrorPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ErrorStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_user_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ErrorStatus {
    Unresolved,
    Resolved,
    Muted,
}

impl ErrorStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unresolved => "unresolved",
            Self::Resolved => "resolved",
            Self::Muted => "muted",
        }
    }
}

// ----- Response types --------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorGroup {
    pub id: String,
    #[serde(default)]
    pub fingerprint_hash: String,
    pub title: String,
    #[serde(default)]
    pub culprit: Option<String>,
    pub environment: String,
    pub level: String,
    pub status: ErrorStatus,
    #[serde(default)]
    pub assigned_user_id: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub event_count: u64,
    pub user_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorPage {
    pub groups: Vec<ErrorGroup>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEvent {
    pub id: String,
    pub group_id: String,
    pub event_id: String,
    pub environment: String,
    pub level: String,
    #[serde(default)]
    pub message: Option<String>,
    pub occurred_at: String,
    /// SDK exception array — usually `[{ "type": …, "value": …, "stacktrace": { "frames": [...] } }]`.
    #[serde(default)]
    pub exception: Option<serde_json::Value>,
    #[serde(default)]
    pub breadcrumbs: Option<serde_json::Value>,
    #[serde(default)]
    pub request: Option<serde_json::Value>,
    #[serde(default)]
    pub user: Option<serde_json::Value>,
    #[serde(default)]
    pub tags: Option<serde_json::Value>,
    #[serde(default)]
    pub contexts: Option<serde_json::Value>,
    #[serde(default)]
    pub sdk: Option<serde_json::Value>,
    #[serde(default)]
    pub release_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorDetail {
    pub group: ErrorGroup,
    #[serde(default)]
    pub recent_events: Vec<ErrorEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEventPage {
    pub events: Vec<ErrorEvent>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

// ----- Stack-frame extraction helper ----------------------------------------

/// One frame in an SDK-shape stacktrace. Optional everywhere — Sentry / OTel
/// payloads are often missing line numbers or context.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct StackFrame {
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub function: Option<String>,
    #[serde(default)]
    pub lineno: Option<i64>,
    #[serde(default)]
    pub colno: Option<i64>,
    #[serde(default)]
    pub abs_path: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub context_line: Option<String>,
    #[serde(default)]
    pub pre_context: Vec<String>,
    #[serde(default)]
    pub post_context: Vec<String>,
}

/// Pulls frames out of the first entry of an SDK `exception` array. Returns
/// `None` when the shape doesn't match — callers render nothing rather than
/// erroring.
pub fn extract_first_stacktrace(
    exception: &Option<serde_json::Value>,
) -> Option<Vec<StackFrame>> {
    let exc = exception.as_ref()?;
    let arr = exc.as_array()?;
    let first = arr.first()?;
    let frames = first.get("stacktrace")?.get("frames")?;
    serde_json::from_value::<Vec<StackFrame>>(frames.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_status_serializes_lowercase() {
        let s = serde_json::to_string(&ErrorStatus::Unresolved).unwrap();
        assert_eq!(s, "\"unresolved\"");
    }

    #[test]
    fn error_list_params_skips_none_fields() {
        let p = ErrorListParams {
            environment: Some("production".into()),
            status: Some(ErrorStatus::Unresolved),
            cursor: None,
            limit: Some(50),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"environment\":\"production\""));
        assert!(s.contains("\"status\":\"unresolved\""));
        assert!(!s.contains("\"cursor\""));
        assert!(s.contains("\"limit\":50"));
    }

    #[test]
    fn extract_first_stacktrace_happy_path() {
        let exc = json!([
            {
                "type": "TypeError",
                "value": "Cannot read properties of undefined",
                "stacktrace": {
                    "frames": [
                        { "function": "doIt", "filename": "foo.ts", "lineno": 5 },
                        { "function": "main", "filename": "main.ts", "lineno": 1 }
                    ]
                }
            }
        ]);
        let frames = extract_first_stacktrace(&Some(exc)).expect("frames");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].function.as_deref(), Some("doIt"));
        assert_eq!(frames[0].lineno, Some(5));
    }

    #[test]
    fn extract_first_stacktrace_returns_none_on_bad_shape() {
        assert!(extract_first_stacktrace(&None).is_none());
        assert!(extract_first_stacktrace(&Some(json!({}))).is_none());
        assert!(extract_first_stacktrace(&Some(json!([]))).is_none());
        assert!(extract_first_stacktrace(&Some(json!([{"other": 1}]))).is_none());
        assert!(
            extract_first_stacktrace(&Some(json!([{"stacktrace": {}}]))).is_none(),
            "missing frames key returns None"
        );
    }

    #[test]
    fn error_group_decodes_minimal_payload() {
        let g: ErrorGroup = serde_json::from_value(json!({
            "id": "abc",
            "title": "Boom",
            "environment": "production",
            "level": "error",
            "status": "unresolved",
            "first_seen_at": "2026-05-23T19:00:00Z",
            "last_seen_at":  "2026-05-23T19:05:00Z",
            "event_count": 12,
            "user_count": 4,
        }))
        .expect("ErrorGroup decodes without optionals");
        assert!(g.culprit.is_none());
        assert_eq!(g.event_count, 12);
    }
}
