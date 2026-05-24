//! Proxy mode — direct Lambda invocation.
//!
//! ADR-017 pivot (see coordinator note attached to SMOODEV-1276/1278):
//! we no longer go through API Gateway. The data plane reconstructs an
//! API-Gateway-proxy-event from the inbound HTTP request, calls
//! `lambda:InvokeFunction` directly, and parses the proxy-response.
//!
//! Trust-boundary attestation lives in
//! `requestContext.authorizer.smooEdge` (the existing Hono middleware
//! can read either header or context.authorizer; Sirius's parallel
//! work updates the TS side).
//!
//! Credential discovery uses the default `aws-config` chain so:
//! - In-cluster: IRSA via the api-prime ServiceAccount.
//! - Local dev: `AWS_PROFILE` env or `AWS_ACCESS_KEY_ID`/etc.

use std::collections::HashMap;

use aws_sdk_lambda::{primitives::Blob, types::InvocationType, Client as LambdaClient};
use base64::{engine::general_purpose::STANDARD as B64_STD, Engine as _};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::edge::auth::EdgeAuthContext;
use crate::edge::cache::CachedResponse;
use crate::edge::edge_attest::EdgeAttestSigner;
use crate::edge::types::RouteEntry;
use crate::error::AppError;

pub struct LambdaProxy {
    client: LambdaClient,
}

impl LambdaProxy {
    pub fn new(client: LambdaClient) -> Self {
        Self { client }
    }

    /// Build the SDK client from the default credential chain (IRSA in
    /// cluster, profile/env locally). Region falls back to `AWS_REGION`
    /// then `us-east-2`.
    pub async fn from_env() -> Self {
        let conf = aws_config::load_from_env().await;
        let region_present = conf.region().is_some();
        let conf = if region_present {
            conf
        } else {
            aws_config::from_env()
                .region(aws_config::Region::new("us-east-2"))
                .load()
                .await
        };
        Self::new(LambdaClient::new(&conf))
    }

    /// Invoke the route's Lambda and parse the response back into a
    /// [`CachedResponse`]. Returns 502 if the Lambda returns a structured
    /// error or a malformed proxy-response.
    pub async fn invoke(
        &self,
        route: &RouteEntry,
        req: &InboundRequest,
        path_params: &HashMap<String, String>,
        auth: &EdgeAuthContext,
        attest: &EdgeAttestSigner,
    ) -> Result<CachedResponse, AppError> {
        let arn = route
            .lambda_arn
            .as_deref()
            .ok_or_else(|| AppError::Internal("route is proxy/cache mode but has no lambdaArn".to_string()))?;

        let event = build_proxy_event(req, path_params, auth, route, attest);
        let payload = serde_json::to_vec(&event).map_err(|e| AppError::Internal(format!("serialize event: {e}")))?;
        let out = self
            .client
            .invoke()
            .function_name(arn)
            .invocation_type(InvocationType::RequestResponse)
            .payload(Blob::new(payload))
            .send()
            .await
            .map_err(|e| {
                warn!(arn = %arn, error = ?e, "lambda invoke failed");
                AppError::Internal(format!("lambda invoke failed: {e}"))
            })?;

        if let Some(err) = out.function_error() {
            let body = out
                .payload()
                .map(|b| String::from_utf8_lossy(b.as_ref()).into_owned())
                .unwrap_or_default();
            warn!(arn = %arn, function_error = %err, body = %body, "lambda returned function error");
            return Err(AppError::Internal(format!("lambda function error: {err}")));
        }

        let resp_bytes = out.payload().map(|b| b.as_ref().to_vec()).unwrap_or_default();
        parse_proxy_response(&resp_bytes)
    }
}

/// Slice of the inbound HTTP request that we need to reconstruct the
/// proxy event. Decoupled from axum types so it's easy to unit-test.
#[derive(Debug, Clone)]
pub struct InboundRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiGatewayProxyEvent {
    resource: String,
    path: String,
    http_method: String,
    headers: HashMap<String, String>,
    multi_value_headers: HashMap<String, Vec<String>>,
    #[serde(serialize_with = "ser_opt_map", deserialize_with = "de_opt_map")]
    query_string_parameters: Option<HashMap<String, String>>,
    #[serde(serialize_with = "ser_opt_map", deserialize_with = "de_opt_map")]
    path_parameters: Option<HashMap<String, String>>,
    body: Option<String>,
    is_base64_encoded: bool,
    request_context: ProxyRequestContext,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyRequestContext {
    request_id: String,
    stage: String,
    authorizer: ProxyAuthorizer,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyAuthorizer {
    /// Edge attestation (HMAC-signed). Sirius's Hono middleware reads
    /// this and skips re-auth/re-ratelimit/re-schema-ingress if valid.
    smoo_edge: crate::edge::edge_attest::EdgeAttestPayload,
    /// Surfaced for handlers that read `authorizer.principalId`.
    principal_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiGatewayProxyResponse {
    status_code: u16,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    is_base64_encoded: bool,
}

fn build_proxy_event(
    req: &InboundRequest,
    path_params: &HashMap<String, String>,
    auth: &EdgeAuthContext,
    route: &RouteEntry,
    attest: &EdgeAttestSigner,
) -> ApiGatewayProxyEvent {
    let mut headers = req.headers.clone();
    if let Some(jwt) = &auth.raw_jwt {
        // Pass the bearer through verbatim so legacy handlers can still
        // call `getUser()` if they really need to.
        headers.insert("Authorization".to_string(), format!("Bearer {jwt}"));
    }
    let multi: HashMap<String, Vec<String>> = headers.iter().map(|(k, v)| (k.clone(), vec![v.clone()])).collect();

    let (body, is_b64) = if req.body.is_empty() {
        (None, false)
    } else if std::str::from_utf8(&req.body).is_ok() {
        (Some(String::from_utf8_lossy(&req.body).into_owned()), false)
    } else {
        (Some(B64_STD.encode(&req.body)), true)
    };

    ApiGatewayProxyEvent {
        resource: route.path.clone(),
        path: req.path.clone(),
        http_method: req.method.to_uppercase(),
        headers,
        multi_value_headers: multi,
        query_string_parameters: if req.query.is_empty() { None } else { Some(req.query.clone()) },
        path_parameters: if path_params.is_empty() {
            None
        } else {
            Some(path_params.clone())
        },
        body,
        is_base64_encoded: is_b64,
        request_context: ProxyRequestContext {
            request_id: uuid::Uuid::new_v4().to_string(),
            stage: std::env::var("STAGE").unwrap_or_else(|_| "production".into()),
            authorizer: ProxyAuthorizer {
                smoo_edge: attest.sign(route, auth),
                principal_id: auth.sub.clone(),
            },
        },
    }
}

fn parse_proxy_response(bytes: &[u8]) -> Result<CachedResponse, AppError> {
    let resp: ApiGatewayProxyResponse = serde_json::from_slice(bytes)
        .map_err(|e| AppError::Internal(format!("malformed lambda proxy response: {e}")))?;
    let headers: Vec<(String, String)> = resp.headers.into_iter().collect();
    let now = crate::edge::cache::now_secs();
    Ok(CachedResponse {
        status: resp.status_code,
        headers,
        body: resp.body,
        is_base64_encoded: resp.is_base64_encoded,
        cached_at: now,
        ttl_at: now,
        swr_at: now,
    })
}

// serde adapters so an empty map serializes as `null` (Hono / API Gateway
// behavior expected by existing handlers).
fn ser_opt_map<S>(v: &Option<HashMap<String, String>>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match v {
        Some(m) => m.serialize(s),
        None => s.serialize_none(),
    }
}

fn de_opt_map<'de, D>(d: D) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<HashMap<String, String>>::deserialize(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::auth::AuthKind;
    use crate::edge::types::{AuthRequirement, RateLimitConfig, RouteMode};

    fn route() -> RouteEntry {
        RouteEntry {
            path: "/orgs/:org_id".into(),
            method: "GET".into(),
            auth: AuthRequirement::User,
            idempotent: true,
            mode: RouteMode::Proxy,
            rate_limit: RateLimitConfig {
                per_token: 1,
                window_seconds: 1,
            },
            cache: None,
            implement: None,
            lambda_arn: Some("arn:fake".into()),
            schema_ref: None,
        }
    }

    #[test]
    fn proxy_event_includes_attestation_in_authorizer() {
        let req = InboundRequest {
            method: "GET".into(),
            path: "/orgs/abc".into(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let mut params = HashMap::new();
        params.insert("org_id".into(), "abc".into());
        let auth = EdgeAuthContext {
            sub: "user-1".into(),
            kind: AuthKind::User { user_id: "user-1".into() },
            raw_jwt: Some("jwt".into()),
        };
        let signer = EdgeAttestSigner::new(*b"secret");
        let event = build_proxy_event(&req, &params, &auth, &route(), &signer);
        assert_eq!(event.http_method, "GET");
        assert_eq!(event.path_parameters.as_ref().unwrap().get("org_id").unwrap(), "abc");
        assert_eq!(event.headers.get("Authorization").unwrap(), "Bearer jwt");
        assert_eq!(event.request_context.authorizer.smoo_edge.claims.sub, "user-1");
        assert!(signer.verify(&event.request_context.authorizer.smoo_edge));
    }

    #[test]
    fn proxy_response_round_trips() {
        let raw = br#"{"statusCode":200,"headers":{"content-type":"application/json"},"body":"{}","isBase64Encoded":false}"#;
        let parsed = parse_proxy_response(raw).unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, "{}");
        assert!(parsed.headers.iter().any(|(k, v)| k == "content-type" && v == "application/json"));
    }
}
