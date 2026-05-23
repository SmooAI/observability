//! Errors view types. Mirror of `packages/backend/src/routes/observability/errors-query.ts`.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ApiError, OrgClient};

impl<'a> OrgClient<'a> {
    pub async fn list_error_groups(
        &self,
        params: &ErrorListParams,
    ) -> Result<ErrorPage, ApiError> {
        self.get("errors", Some(params)).await
    }

    pub async fn get_error_group(&self, group_id: Uuid) -> Result<ErrorDetail, ApiError> {
        self.get(&format!("errors/{group_id}"), Option::<&()>::None).await
    }

    pub async fn list_group_events(
        &self,
        group_id: Uuid,
        params: &PageParams,
    ) -> Result<ErrorEventPage, ApiError> {
        self.get(&format!("errors/{group_id}/events"), Some(params)).await
    }

    pub async fn update_error_group(
        &self,
        group_id: Uuid,
        patch: &ErrorPatch,
    ) -> Result<ErrorGroup, ApiError> {
        self.patch(&format!("errors/{group_id}"), patch).await
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PageParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEventPage {
    pub events: Vec<ErrorEvent>,
    pub next_cursor: Option<String>,
}

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

#[derive(Debug, Clone, Default, Serialize)]
pub struct ErrorPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ErrorStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_user_id: Option<String>,
}
