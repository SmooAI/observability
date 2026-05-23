//! Bare `client_credentials` exchange against `https://auth.smoo.ai/token`.
//!
//! The shape matches `packages/auth/src/server/ClientCredentialsProvider.ts`:
//!
//! ```text
//! POST https://auth.smoo.ai/token
//! Content-Type: application/x-www-form-urlencoded
//!
//! grant_type=client_credentials
//! provider=client_credentials
//! client_id=<uuid>
//! client_secret=sk_<base64>
//! ```
//!
//! Returns `{ access_token, token_type, expires_in }` (TTL = 3600s today).

use super::{AuthError, TokenResponse, TOKEN_URL};

#[derive(Clone)]
pub struct OAuthClient {
    http: reqwest::Client,
    token_url: String,
}

impl OAuthClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            token_url: TOKEN_URL.to_string(),
        }
    }

    /// Override the token URL for testing.
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    pub async fn exchange(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, AuthError> {
        let resp = self
            .http
            .post(&self.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("provider", "client_credentials"),
                ("client_id", client_id),
                ("client_secret", client_secret),
            ])
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            // Drain body for nicer error logging; ignore decode failures.
            let _ = resp.text().await;
            return Err(AuthError::TokenEndpoint(status));
        }

        Ok(resp.json::<TokenResponse>().await?)
    }
}
