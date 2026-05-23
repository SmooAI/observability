//! Typed client for `https://api.smoo.ai/organizations/{org_id}/observability/*`.
//!
//! Response types mirror `apps/web/components/services/observability-service.ts`
//! 1:1 so updates to the canonical browser dashboard apply here with minimal
//! translation cost. See
//! `docs/Engineering/Rust-Desktop-Observability-Viewer.md` §5.2.

use serde::{de::DeserializeOwned, Serialize};
use url::Url;
use uuid::Uuid;

use crate::auth::{AuthError, AuthManager, API_BASE};

pub mod connections;
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
    Status {
        status: reqwest::StatusCode,
        body: String,
    },
}

/// Single shared API client. Holds the reqwest pool + auth handle; everything
/// per-org flows through the typed `OrgClient` view returned by `org()`.
///
/// Used by views in phases 3+; declared up front so the headless auth + API
/// layer can be tested end-to-end ahead of UI wiring.
#[derive(Clone)]
#[allow(dead_code)]
pub struct ApiClient {
    http: reqwest::Client,
    auth: AuthManager,
    base: Url,
}

#[allow(dead_code)] // `with_base` + `org` are used by integration tests and phase 3+ views.
impl ApiClient {
    pub fn new(http: reqwest::Client, auth: AuthManager) -> Result<Self, ApiError> {
        Ok(Self {
            base: Url::parse(API_BASE)?,
            http,
            auth,
        })
    }

    /// Override the API base URL for testing or staging.
    pub fn with_base(mut self, base: impl reqwest::IntoUrl) -> Result<Self, ApiError> {
        self.base = Url::parse(base.as_str())?;
        Ok(self)
    }

    pub fn org(&self, org_id: Uuid) -> OrgClient<'_> {
        OrgClient { client: self, org_id }
    }
}

/// Per-org typed view. Composes the URL path, injects the bearer, and retries
/// once on 401 (which fires after the cached token has been invalidated).
pub struct OrgClient<'a> {
    client: &'a ApiClient,
    org_id: Uuid,
}

impl<'a> OrgClient<'a> {
    fn endpoint(&self, path: &str) -> Result<Url, ApiError> {
        // `path` is appended verbatim after `/organizations/{org}/observability`.
        let base = self.client.base.join(&format!(
            "/organizations/{org}/observability/",
            org = self.org_id
        ))?;
        Ok(base.join(path.trim_start_matches('/'))?)
    }

    /// GET with optional query params.
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

    pub async fn delete(&self, path: &str) -> Result<(), ApiError> {
        let _: serde_json::Value = self
            .send(reqwest::Method::DELETE, path, Option::<&()>::None, Option::<&()>::None)
            .await
            .or_else(|e| match e {
                // 204 No Content → reqwest fails to decode JSON; map to ()
                ApiError::Http(ref re) if re.is_decode() => Ok(serde_json::Value::Null),
                other => Err(other),
            })?;
        Ok(())
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

    fn dummy_auth() -> AuthManager {
        AuthManager::new(reqwest::Client::new())
    }

    #[test]
    fn endpoint_composition_includes_org_and_path() {
        let api = ApiClient::new(reqwest::Client::new(), dummy_auth())
            .unwrap()
            .with_base("https://api.example.test")
            .unwrap();
        let org = Uuid::nil();
        let url = api.org(org).endpoint("logs/query").unwrap();
        let expected = format!(
            "https://api.example.test/organizations/{org}/observability/logs/query"
        );
        assert_eq!(url.as_str(), expected);
    }

    #[test]
    fn endpoint_strips_leading_slash() {
        let api = ApiClient::new(reqwest::Client::new(), dummy_auth())
            .unwrap()
            .with_base("https://api.example.test")
            .unwrap();
        let org = Uuid::nil();
        let with = api.org(org).endpoint("/errors/abc").unwrap();
        let without = api.org(org).endpoint("errors/abc").unwrap();
        assert_eq!(with, without);
    }
}
