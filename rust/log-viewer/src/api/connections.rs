//! Settings view types — `/organizations/{org_id}/observability/connect[ion]`.
//! Mirror of `packages/backend/src/routes/observability/connections.ts`.
//!
//! Filled in during phase 2 of SMOODEV-1175 alongside the auth wiring.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize)]
pub struct InitiateConnection {
    pub aws_account_id: Option<String>,
    pub aws_regions: Option<Vec<String>>,
    pub role_arn: Option<String>,
    pub log_group_filters: Option<Vec<String>>,
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
