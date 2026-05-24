//! Fetch SST stack outputs JSON from S3.
//!
//! SST writes its stack state to `s3://${SST_STATE_BUCKET}/app/smooai/${STAGE}.json`
//! after each deploy. We pull this on every reconcile and use it to resolve
//! per-route `lambdaOutputKey` → ARN. See ADR-017 §"Reconcile loop".

use std::collections::HashMap;

use anyhow::{Context, Result};
use aws_sdk_s3::Client as S3Client;

/// Trait abstracting the S3 fetch so reconcile tests can pass a stub instead
/// of a real S3 client. Production controllers wire [`S3OutputsFetcher`].
#[async_trait::async_trait]
pub trait SstOutputsFetcher: Send + Sync {
    async fn fetch(&self) -> Result<HashMap<String, String>>;
}

pub struct S3OutputsFetcher {
    client: S3Client,
    bucket: String,
    key: String,
}

impl S3OutputsFetcher {
    pub fn new(client: S3Client, bucket: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
            key: key.into(),
        }
    }

    /// Convenience constructor that derives the standard SST key from a
    /// stage name (`app/smooai/<stage>.json`).
    pub fn from_stage(client: S3Client, bucket: impl Into<String>, stage: &str) -> Self {
        Self::new(client, bucket, format!("app/smooai/{}.json", stage))
    }
}

#[async_trait::async_trait]
impl SstOutputsFetcher for S3OutputsFetcher {
    async fn fetch(&self) -> Result<HashMap<String, String>> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .send()
            .await
            .with_context(|| format!("GetObject s3://{}/{} failed", self.bucket, self.key))?;

        let bytes = resp
            .body
            .collect()
            .await
            .context("failed to collect SST outputs body")?
            .into_bytes();

        parse_outputs(&bytes)
    }
}

/// Parse the SST outputs JSON into a flat `name -> value` map.
///
/// SST's state file has the shape:
/// ```json
/// {
///   "version": 1,
///   "outputs": { "ApiRouteFoo": "arn:aws:lambda:...", "WebsiteUrl": "..." }
/// }
/// ```
/// We accept both that envelope and a bare `{name: value}` map so legacy
/// fixtures don't need updating.
pub fn parse_outputs(bytes: &[u8]) -> Result<HashMap<String, String>> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        outputs: serde_json::Value,
    }

    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("SST outputs JSON failed to parse")?;

    // Prefer the envelope form if present.
    let outputs = if let Ok(env) = serde_json::from_value::<Envelope>(value.clone()) {
        env.outputs
    } else {
        value
    };

    let obj = outputs
        .as_object()
        .context("SST outputs is not a JSON object")?;
    let mut map = HashMap::with_capacity(obj.len());
    for (k, v) in obj {
        if let Some(s) = v.as_str() {
            map.insert(k.clone(), s.to_string());
        }
        // Non-string outputs (numbers, nested objects) are not resolvable to
        // Lambda ARNs; skip them silently.
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_envelope_form() {
        let json = br#"{"version":1,"outputs":{"ApiRouteFoo":"arn:aws:lambda:us-east-1:123:function:foo","Other":42}}"#;
        let map = parse_outputs(json).unwrap();
        assert_eq!(
            map.get("ApiRouteFoo").map(String::as_str),
            Some("arn:aws:lambda:us-east-1:123:function:foo")
        );
        assert!(!map.contains_key("Other"), "non-string output should be skipped");
    }

    #[test]
    fn parses_bare_object() {
        let json = br#"{"ApiRouteFoo":"arn:1","ApiRouteBar":"arn:2"}"#;
        let map = parse_outputs(json).unwrap();
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn rejects_non_object() {
        assert!(parse_outputs(b"[\"not\",\"obj\"]").is_err());
    }
}
