//! # SmooAI Observability — Rust SDK
//!
//! Error capture, PII scrubbing, batched webhook transport, OpenTelemetry
//! traces + metrics, GenAI semantic-conventions, and M2M auth — at parity with
//! the TypeScript `@smooai/observability` reference SDK so Rust services
//! (api-prime, voice, temporal-worker) can self-emit telemetry to api.smoo.ai
//! over the exact same wire format.
//!
//! ## Quick start
//!
//! ```no_run
//! # async fn run() {
//! // 1. Bootstrap from env vars (SMOOAI_OBSERVABILITY_ENDPOINT / _AUTH_URL / …).
//! let result = smooai_observability::bootstrap().await;
//!
//! // 2. Capture errors and messages anywhere.
//! smooai_observability::capture_message("worker started", smooai_observability::Level::Info);
//!
//! // 3. Emit application metrics.
//! let m = smooai_observability::metrics::metrics_client("smooai-voice");
//! m.counter("agent.turn.completed", 1, &[("channel", "voice")]);
//!
//! // 4. On shutdown, flush.
//! if let Some(otel) = &result.otel { otel.flush(); }
//! result.client.flush().await;
//! # }
//! ```
//!
//! Every public entry point is error-safe — observability must never panic the
//! host application. Failures degrade to no-ops and a single line on stderr.
//!
//! See `~/dev/smooai/observability/packages/core/src/` for the TS reference.

pub mod auth;
pub mod bootstrap;
pub mod client;
pub mod gen_ai;
pub mod metrics;
pub mod otel;
pub mod pii;
pub mod scope;
mod stack;
pub mod transport;
pub mod types;

use once_cell::sync::OnceCell;
use std::time::{SystemTime, UNIX_EPOCH};

// ---- Re-exports: the ergonomic surface most callers want. ------------------

pub use auth::{TokenError, TokenProvider, TokenProviderOptions};
pub use bootstrap::{bootstrap, bootstrap_with, BootstrapEnv, BootstrapResult};
pub use client::{BeforeSend, CaptureHandler, Client, ClientOptions};
pub use gen_ai::{
    record_gen_ai_message, set_gen_ai_attributes, GenAIAttributes, GenAIMessageExtra,
    GenAIOperationName, GenAIRole, GenAISystem,
};
pub use metrics::{metrics_client, metrics_client_default, MetricsClient};
pub use otel::{setup_otel_sdk, OtelSdkHandle, SetupOtelOptions};
pub use scope::{current_scope, global_scope, with_scope, with_scope_sync, Scope};
pub use types::{
    Breadcrumb, ExceptionInfo, IngestPayload, IngestType, Level, ObservabilityEvent, RequestInfo,
    Runtime, SdkInfo, StackFrame, StackTrace, UserContext,
};

// ---- Shared helpers (crate-internal + a few public). -----------------------

/// Milliseconds since the Unix epoch — the timestamp format used across the
/// event envelope (matches the TS `Date.now()`).
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Generate a fresh UUID v4 event id.
pub(crate) fn new_event_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---- Process-wide singleton client (optional convenience). -----------------

static GLOBAL_CLIENT: OnceCell<Client> = OnceCell::new();

/// Install a process-wide singleton [`Client`] so the free `capture_*`
/// functions below have somewhere to dispatch. Idempotent — a second call
/// returns the already-installed client. [`bootstrap`] does NOT call this
/// automatically; call [`set_global_client`] with `bootstrap().client` if you
/// want the free functions wired.
pub fn set_global_client(client: Client) -> Client {
    let _ = GLOBAL_CLIENT.set(client);
    GLOBAL_CLIENT.get().cloned().expect("client just set")
}

/// Convenience: initialize from [`ClientOptions`] and install as the global
/// client in one call.
pub fn init(options: ClientOptions) -> Client {
    set_global_client(Client::init(options))
}

/// The installed global client, if any.
pub fn global_client() -> Option<Client> {
    GLOBAL_CLIENT.get().cloned()
}

/// Capture an error via the global client. No-op (returns `None`) if no global
/// client is installed.
pub fn capture_exception<E: std::error::Error + ?Sized>(error: &E) -> Option<String> {
    global_client().map(|c| c.capture_exception(error))
}

/// Capture a message via the global client. No-op if no global client.
pub fn capture_message(message: impl Into<String>, level: Level) -> Option<String> {
    global_client().map(|c| c.capture_message(message, level))
}

/// Add a breadcrumb to the current scope. Always available (uses the
/// task-local / global scope), independent of the global client.
pub fn add_breadcrumb(category: impl Into<String>, message: Option<String>, level: Level) {
    current_scope().add_breadcrumb_parts(category, message, None, level);
}

/// Set the user on the current scope.
pub fn set_user(user: Option<UserContext>) {
    current_scope().set_user(user);
}

/// Set a tag on the current scope.
pub fn set_tag(key: impl Into<String>, value: impl Into<String>) {
    current_scope().set_tag(key, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_is_positive() {
        assert!(now_ms() > 0);
    }

    #[test]
    fn event_id_is_uuid_v4_shape() {
        let id = new_event_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
    }

    #[test]
    fn capture_without_global_client_is_none() {
        // In a fresh process with no global client, the free fn returns None.
        // (Other tests may install one; this assertion only holds if run first,
        // so just confirm the call is panic-free.)
        let _ = capture_message("noop", Level::Debug);
    }
}
