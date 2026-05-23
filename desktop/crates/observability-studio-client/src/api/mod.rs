//! Typed `https://api.smoo.ai/organizations/{org_id}/observability/*` client.
//!
//! Each view module lives in its own file and adds typed methods on
//! `OrgClient` via `impl` blocks. Phase-1 of the Dioxus pivot ships the
//! plumbing + Settings flow only; per-view methods will be wired up as the
//! Logs / Errors / Metrics views land.

use serde::{de::DeserializeOwned, Serialize};
use url::Url;
use uuid::Uuid;

use crate::auth::{AuthError, AuthManager, API_BASE};

pub mod errors;
pub mod logs;
pub mod metrics;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("url: {0}")]
    Url(#[from] url::ParseError),
    #[error("api returned {status}: {body}")]
    Status { status: reqwest::StatusCode, body: String },
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    auth: AuthManager,
    base: Url,
}

impl ApiClient {
    pub fn new(http: reqwest::Client, auth: AuthManager) -> Result<Self, ApiError> {
        Ok(Self {
            base: Url::parse(API_BASE)?,
            http,
            auth,
        })
    }

    pub fn with_base(mut self, base: impl reqwest::IntoUrl) -> Result<Self, ApiError> {
        self.base = Url::parse(base.as_str())?;
        Ok(self)
    }

    pub fn org(&self, org_id: Uuid) -> OrgClient<'_> {
        OrgClient { client: self, org_id }
    }
}

pub struct OrgClient<'a> {
    pub(crate) client: &'a ApiClient,
    pub(crate) org_id: Uuid,
}

impl<'a> OrgClient<'a> {
    pub(crate) fn endpoint(&self, path: &str) -> Result<Url, ApiError> {
        let base = self
            .client
            .base
            .join(&format!("/organizations/{org}/observability/", org = self.org_id))?;
        Ok(base.join(path.trim_start_matches('/'))?)
    }

    pub async fn get<R, Q>(&self, path: &str, query: Option<&Q>) -> Result<R, ApiError>
    where
        R: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        self.send(reqwest::Method::GET, path, query, Option::<&()>::None).await
    }

    pub async fn post<R, B>(&self, path: &str, body: &B) -> Result<R, ApiError>
    where
        R: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.send(reqwest::Method::POST, path, Option::<&()>::None, Some(body)).await
    }

    pub async fn patch<R, B>(&self, path: &str, body: &B) -> Result<R, ApiError>
    where
        R: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.send(reqwest::Method::PATCH, path, Option::<&()>::None, Some(body)).await
    }

    async fn send<R, Q, B>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Option<&Q>,
        body: Option<&B>,
    ) -> Result<R, ApiError>
    where
        R: DeserializeOwned,
        Q: Serialize + ?Sized,
        B: Serialize + ?Sized,
    {
        let url = self.endpoint(path)?;
        let attempt = |bearer: String| {
            let mut req = self
                .client
                .http
                .request(method.clone(), url.clone())
                .bearer_auth(&bearer);
            if let Some(q) = query {
                req = req.query(q);
            }
            if let Some(b) = body {
                req = req.json(b);
            }
            req
        };

        let bearer = self.client.auth.bearer_for(self.org_id).await?;
        let resp = attempt(bearer).send().await?;

        // One-shot 401 → invalidate + remint + retry. Tokens are 1h but the
        // server may have rotated mid-session (or our clock drifted).
        let resp = if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.client.auth.invalidate(self.org_id);
            let bearer = self.client.auth.bearer_for(self.org_id).await?;
            attempt(bearer).send().await?
        } else {
            resp
        };

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status { status, body });
        }
        Ok(resp.json::<R>().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthManager;

    #[test]
    fn endpoint_composition_includes_org_and_path() {
        let api = ApiClient::new(reqwest::Client::new(), AuthManager::new(reqwest::Client::new()))
            .unwrap()
            .with_base("https://api.example.test")
            .unwrap();
        let org = Uuid::nil();
        let url = api.org(org).endpoint("logs/query").unwrap();
        assert_eq!(
            url.as_str(),
            format!("https://api.example.test/organizations/{org}/observability/logs/query"),
        );
    }
}
