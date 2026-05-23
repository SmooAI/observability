//! M2M `client_credentials` auth manager: exchanges `client_id`/`client_secret`
//! at `https://auth.smoo.ai/token`, caches the bearer JWT (1h TTL), refreshes
//! 60s before expiry. Credentials live in the OS keychain via the `keyring`
//! crate.
//!
//! Filled in during phase 2 of SMOODEV-1175. See
//! `docs/Engineering/Rust-Desktop-Observability-Viewer.md` §5.3.

#![allow(dead_code)]

use serde::Deserialize;

pub const TOKEN_URL: &str = "https://auth.smoo.ai/token";
pub const API_BASE: &str = "https://api.smoo.ai";

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credentials stored for org {0}")]
    MissingCredentials(uuid::Uuid),
    #[error("token endpoint returned status {0}")]
    TokenEndpoint(reqwest::StatusCode),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
}
