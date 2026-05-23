//! Errors view types. Mirror of `packages/backend/src/routes/observability/errors-query.ts`.
//!
//! Filled in during phase 4 of SMOODEV-1175.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct ErrorListParams {
    pub environment: Option<String>,
    pub status: Option<ErrorStatus>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ErrorStatus {
    Unresolved,
    Resolved,
    Muted,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorGroup {
    pub id: String,
    pub fingerprint_hash: String,
    pub title: String,
    pub culprit: Option<String>,
    pub environment: String,
    pub level: String,
    pub status: ErrorStatus,
    pub assigned_user_id: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub event_count: u64,
    pub user_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorPage {
    pub groups: Vec<ErrorGroup>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEvent {
    pub id: String,
    pub group_id: String,
    pub event_id: String,
    pub environment: String,
    pub level: String,
    pub message: Option<String>,
    pub occurred_at: String,
    pub exception: Option<serde_json::Value>,
    pub breadcrumbs: Option<serde_json::Value>,
    pub request: Option<serde_json::Value>,
    pub user: Option<serde_json::Value>,
    pub tags: Option<serde_json::Value>,
    pub contexts: Option<serde_json::Value>,
    pub sdk: Option<serde_json::Value>,
    pub release_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorDetail {
    pub group: ErrorGroup,
    pub recent_events: Vec<ErrorEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorPatch {
    pub status: Option<ErrorStatus>,
    pub assigned_user_id: Option<String>,
}
