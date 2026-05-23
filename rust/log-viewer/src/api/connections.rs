//! Settings view types — `/organizations/{org_id}/observability/connect[ion]`.
//! Mirror of `packages/backend/src/routes/observability/connections.ts`.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use super::{ApiError, OrgClient};

impl<'a> OrgClient<'a> {
    pub async fn initiate_connection(
        &self,
        body: &InitiateConnection,
    ) -> Result<InitiateConnectionResponse, ApiError> {
        self.post("connect", body).await
    }

    /// `GET /connection` — backend returns `null` (JSON null) when the org has
    /// no connection yet. We model that as `Option`.
    pub async fn get_connection(&self) -> Result<Option<ObservabilityConnection>, ApiError> {
        self.get("connection", Option::<&()>::None).await
    }

    pub async fn update_connection(
        &self,
        body: &UpdateConnection,
    ) -> Result<ObservabilityConnection, ApiError> {
        self.patch("connection", body).await
    }

    pub async fn disconnect(&self) -> Result<(), ApiError> {
        self.delete("connection").await
    }

    pub async fn verify_connection(&self) -> Result<VerifyConnectionResponse, ApiError> {
        // Backend route: POST /connection/verify, no body.
        self.post("connection/verify", &serde_json::json!({})).await
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateConnection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_regions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_filters: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionStatus {
    Active,
    Inactive,
    Error,
    Pending,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservabilityConnection {
    pub id: String,
    pub organization_id: String,
    pub status: ConnectionStatus,
    pub aws_account_id: String,
    pub aws_regions: Vec<String>,
    pub role_arn: String,
    pub stack_id: Option<String>,
    pub log_group_filters: Option<Vec<String>>,
    pub last_sync_at: Option<String>,
    pub error_message: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InitiateConnection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_regions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_filters: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploy_region: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InitiateConnectionResponse {
    pub quick_create_url: String,
    pub connection_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerifyConnectionResponse {
    pub success: bool,
    pub status: ConnectionStatus,
    pub message: String,
}
