//! One-call, env-driven bootstrap — mirrors the TS `bootstrap/index.ts`.
//!
//! A downstream Rust service calls [`bootstrap`] once near startup (typically
//! right after the tokio runtime is up). It reads config from environment
//! variables — no schema imports, no Smoo-internal coupling — wires the OTel
//! SDK (traces + metrics) and the capture [`Client`], and returns a handle with
//! flush/shutdown hooks.
//!
//! ## Env vars (identical names to the TS SDK)
//!
//! - `SMOOAI_OBSERVABILITY_ENDPOINT` — base ingest URL (e.g. `https://api.smoo.ai`).
//!   `/v1/traces`, `/v1/metrics`, and `/v1/logs` are appended. Per-signal
//!   overrides via the standard `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` /
//!   `_METRICS_ENDPOINT` / `_LOGS_ENDPOINT`.
//! - Auth (pick one; pre-minted token wins):
//!   - `SMOOAI_OBSERVABILITY_TOKEN` — pre-minted Bearer JWT (not refreshed).
//!   - `SMOOAI_OBSERVABILITY_AUTH_URL` + `_CLIENT_ID` + `_CLIENT_SECRET` —
//!     `client_credentials` grant, refreshed per-request by the exporter.
//! - `SMOOAI_OBSERVABILITY_SERVICE_NAME` — default `smoo-service`.
//! - `SMOOAI_OBSERVABILITY_ENVIRONMENT` — default `STAGE` / `unknown`.
//! - `SMOOAI_OBSERVABILITY_RELEASE` — default `GIT_SHA` / `dev`.
//! - `SMOOAI_OBSERVABILITY_DISABLED` — `1`/`true` skips bootstrap entirely.
//!
//! Never panics: missing config / init errors are logged to stderr and the SDK
//! degrades gracefully. Idempotent: a second call returns the same handle.

use crate::auth::{TokenProvider, TokenProviderOptions};
use crate::client::{Client, ClientOptions};
use crate::otel::{setup_otel_sdk, OtelSdkHandle, SetupOtelOptions};
use once_cell::sync::OnceCell;
use std::env;

/// Resolved bootstrap config. Exposed so tests + advanced callers can construct
/// it directly instead of going through the environment.
#[derive(Clone, Default)]
pub struct BootstrapEnv {
    pub endpoint: Option<String>,
    pub traces_endpoint: Option<String>,
    pub metrics_endpoint: Option<String>,
    pub logs_endpoint: Option<String>,
    pub token: Option<String>,
    pub auth_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub service_name: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub disabled: bool,
}

impl BootstrapEnv {
    /// Read every field from the environment, applying the same defaults as the
    /// TS bootstrap.
    pub fn from_env() -> Self {
        BootstrapEnv {
            endpoint: env::var("SMOOAI_OBSERVABILITY_ENDPOINT").ok(),
            traces_endpoint: env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").ok(),
            metrics_endpoint: env::var("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT").ok(),
            logs_endpoint: env::var("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT").ok(),
            token: env::var("SMOOAI_OBSERVABILITY_TOKEN").ok(),
            auth_url: env::var("SMOOAI_OBSERVABILITY_AUTH_URL").ok(),
            client_id: env::var("SMOOAI_OBSERVABILITY_CLIENT_ID").ok(),
            client_secret: env::var("SMOOAI_OBSERVABILITY_CLIENT_SECRET").ok(),
            service_name: env::var("SMOOAI_OBSERVABILITY_SERVICE_NAME").ok(),
            environment: env::var("SMOOAI_OBSERVABILITY_ENVIRONMENT")
                .ok()
                .or_else(|| env::var("STAGE").ok()),
            release: env::var("SMOOAI_OBSERVABILITY_RELEASE")
                .ok()
                .or_else(|| env::var("GIT_SHA").ok()),
            disabled: truthy(env::var("SMOOAI_OBSERVABILITY_DISABLED").ok().as_deref()),
        }
    }
}

/// Result of [`bootstrap`].
#[derive(Clone)]
pub struct BootstrapResult {
    /// Whether the bootstrap actually ran (false = disabled or already-installed-elsewhere).
    pub installed: bool,
    /// OTel handle — flush/shutdown. `None` if no endpoint was configured.
    pub otel: Option<OtelSdkHandle>,
    /// The capture client. Always present; capture-handler-only if no DSN.
    pub client: Client,
}

static BOOTSTRAPPED: OnceCell<BootstrapResult> = OnceCell::new();

/// Bootstrap from environment variables. See the module docs for the var list.
/// Idempotent.
pub async fn bootstrap() -> BootstrapResult {
    bootstrap_with(BootstrapEnv::from_env()).await
}

/// Bootstrap from an explicit [`BootstrapEnv`] (test / advanced seam). Async so
/// the initial token mint can warm before the exporter is built.
pub async fn bootstrap_with(env: BootstrapEnv) -> BootstrapResult {
    if let Some(existing) = BOOTSTRAPPED.get() {
        return existing.clone();
    }

    let result = build(env).await;
    let _ = BOOTSTRAPPED.set(result.clone());
    BOOTSTRAPPED.get().cloned().unwrap_or(result)
}

async fn build(env: BootstrapEnv) -> BootstrapResult {
    let service_name = env
        .service_name
        .clone()
        .unwrap_or_else(|| "smoo-service".to_string());
    let environment = env.environment.clone();
    let release = env.release.clone().or_else(|| Some("dev".to_string()));

    if env.disabled {
        return BootstrapResult {
            installed: false,
            otel: None,
            client: Client::init(ClientOptions::default()),
        };
    }

    let traces_endpoint = env.traces_endpoint.clone().or_else(|| {
        env.endpoint
            .as_ref()
            .map(|e| format!("{}/v1/traces", strip_trailing_slash(e)))
    });
    let metrics_endpoint = env.metrics_endpoint.clone().or_else(|| {
        env.endpoint
            .as_ref()
            .map(|e| format!("{}/v1/metrics", strip_trailing_slash(e)))
    });
    let logs_endpoint = env.logs_endpoint.clone().or_else(|| {
        env.endpoint
            .as_ref()
            .map(|e| format!("{}/v1/logs", strip_trailing_slash(e)))
    });

    // Auth: static token wins; otherwise build a per-request TokenProvider and
    // warm it so the first export doesn't pay the round-trip.
    let mut static_headers = std::collections::HashMap::new();
    let mut token_provider: Option<TokenProvider> = None;

    if let Some(token) = &env.token {
        static_headers.insert("authorization".to_string(), format!("Bearer {token}"));
    } else if let (Some(auth_url), Some(client_id), Some(client_secret)) =
        (&env.auth_url, &env.client_id, &env.client_secret)
    {
        match TokenProvider::new(TokenProviderOptions::new(
            auth_url.clone(),
            client_id.clone(),
            client_secret.clone(),
        )) {
            Ok(tp) => {
                if let Err(e) = tp.get_access_token().await {
                    warn(&format!(
                        "initial token mint failed; exports will retry: {e}"
                    ));
                }
                token_provider = Some(tp);
            }
            Err(e) => warn(&format!("token provider config invalid: {e}")),
        }
    } else {
        warn("no auth configured (set SMOOAI_OBSERVABILITY_TOKEN or _AUTH_URL/_CLIENT_ID/_CLIENT_SECRET); OTLP exports will be unauthenticated");
    }

    let otel = if traces_endpoint.is_some() || metrics_endpoint.is_some() || logs_endpoint.is_some()
    {
        let mut opts = SetupOtelOptions::new(service_name);
        opts.otlp_traces_endpoint = traces_endpoint;
        opts.otlp_metrics_endpoint = metrics_endpoint;
        opts.otlp_logs_endpoint = logs_endpoint;
        opts.otlp_headers = static_headers;
        opts.environment = environment.clone();
        opts.release = release.clone();
        opts.token_provider = token_provider;
        Some(setup_otel_sdk(opts))
    } else {
        None
    };

    let client = Client::init(ClientOptions {
        dsn: env::var("OBSERVABILITY_DSN").unwrap_or_default(),
        environment,
        release,
        ..Default::default()
    });

    BootstrapResult {
        installed: true,
        otel,
        client,
    }
}

fn strip_trailing_slash(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn truthy(s: Option<&str>) -> bool {
    matches!(s.map(|v| v.to_lowercase()), Some(ref v) if v == "1" || v == "true")
}

fn warn(message: &str) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr(),
        "[@smooai/observability/bootstrap] {message}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_parses() {
        assert!(truthy(Some("1")));
        assert!(truthy(Some("true")));
        assert!(truthy(Some("TRUE")));
        assert!(!truthy(Some("0")));
        assert!(!truthy(Some("no")));
        assert!(!truthy(None));
    }

    #[test]
    fn strip_trailing_slash_works() {
        assert_eq!(
            strip_trailing_slash("https://api.smoo.ai/"),
            "https://api.smoo.ai"
        );
        assert_eq!(
            strip_trailing_slash("https://api.smoo.ai"),
            "https://api.smoo.ai"
        );
    }

    #[tokio::test]
    async fn disabled_env_skips_otel() {
        let env = BootstrapEnv {
            disabled: true,
            ..Default::default()
        };
        let result = build(env).await;
        assert!(!result.installed);
        assert!(result.otel.is_none());
    }

    #[tokio::test]
    async fn no_endpoint_yields_no_otel_but_a_client() {
        let env = BootstrapEnv {
            service_name: Some("svc".into()),
            ..Default::default()
        };
        let result = build(env).await;
        assert!(result.installed);
        assert!(result.otel.is_none());
        // capture client still usable
        result
            .client
            .capture_message("hi", crate::types::Level::Info);
    }
}
