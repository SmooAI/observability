//! Capture client — `capture_exception` / `capture_message`, mirroring the TS
//! `client.ts`.
//!
//! Builds an [`ObservabilityEvent`] from an error or message, merges the
//! current [`Scope`](crate::scope::Scope), scrubs PII, applies an optional
//! `before_send` hook, and dispatches it to:
//!   - the batched HTTP transport (webhook → Errors dashboard), and/or
//!   - a runtime-native capture handler (e.g. OTel span events).
//!
//! Matches the TS SMOODEV-1148 "fire BOTH paths" behavior: when both a
//! transport and a capture handler are registered, both receive the event.
//!
//! Errors are swallowed everywhere — capture must never panic the host.

use crate::scope::current_scope;
use crate::transport::{Transport, TransportOptions};
use crate::types::{ExceptionInfo, Level, ObservabilityEvent, Runtime, SdkInfo, StackTrace};
use std::sync::Arc;

pub const SDK_NAME: &str = "@smooai/observability";
pub const SDK_VERSION: &str = "0.1.0";

/// A `before_send` predicate. Return `Some(event)` to send (possibly mutated),
/// or `None` to drop the event (e.g. known noise). Mirrors the TS
/// `beforeSend`.
pub type BeforeSend = Arc<dyn Fn(ObservabilityEvent) -> Option<ObservabilityEvent> + Send + Sync>;

/// A runtime-native capture handler — e.g. write an OTel span event. Invoked IN
/// ADDITION to the HTTP transport (SMOODEV-1148).
pub type CaptureHandler = Arc<dyn Fn(&ObservabilityEvent) + Send + Sync>;

/// Options for [`Client::init`]. Mirrors the TS `ClientOptions` (server subset).
#[derive(Clone, Default)]
pub struct ClientOptions {
    /// Ingest endpoint: `POST /webhooks/observability/{org_id}/{token}`. Empty
    /// disables the HTTP transport (capture-handler-only mode).
    pub dsn: String,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub max_queue_size: Option<usize>,
    pub flush_interval_ms: Option<u64>,
    pub max_batch_size: Option<usize>,
    pub before_send: Option<BeforeSend>,
}

#[derive(Clone)]
struct ClientState {
    options: ClientOptions,
    transport: Option<Transport>,
    capture_handler: Option<CaptureHandler>,
}

/// The capture client. Cheap to clone (`Arc`-shared). Construct with
/// [`Client::init`]; for a process-wide singleton use [`crate::init`] /
/// [`crate::global_client`].
#[derive(Clone)]
pub struct Client {
    state: Arc<ClientState>,
}

impl Client {
    /// Initialize a client. If `dsn` is non-empty a batched HTTP transport is
    /// spawned (requires a tokio runtime). Otherwise the client is
    /// capture-handler-only.
    pub fn init(options: ClientOptions) -> Self {
        let transport = if options.dsn.is_empty() {
            None
        } else {
            let mut t = TransportOptions::new(options.dsn.clone());
            if let Some(q) = options.max_queue_size {
                t.max_queue_size = q;
            }
            if let Some(ms) = options.flush_interval_ms {
                t.flush_interval = std::time::Duration::from_millis(ms);
            }
            if let Some(b) = options.max_batch_size {
                t.max_batch_size = b;
            }
            Some(Transport::new(t))
        };
        Client {
            state: Arc::new(ClientState {
                options,
                transport,
                capture_handler: None,
            }),
        }
    }

    /// Return a clone with a runtime-native capture handler attached (e.g. OTel
    /// span events). The handler fires in addition to the transport.
    pub fn with_capture_handler(&self, handler: CaptureHandler) -> Self {
        let mut state = (*self.state).clone();
        state.capture_handler = Some(handler);
        Client {
            state: Arc::new(state),
        }
    }

    fn sdk_info() -> SdkInfo {
        SdkInfo {
            name: SDK_NAME.to_string(),
            version: SDK_VERSION.to_string(),
            runtime: Runtime::Node,
        }
    }

    /// Capture an error. Returns the generated event id. Builds the exception
    /// chain (via [`std::error::Error::source`]), captures a stack, merges the
    /// current scope, scrubs PII, runs `before_send`, and dispatches.
    pub fn capture_exception<E: std::error::Error + ?Sized>(&self, error: &E) -> String {
        let event_id = crate::new_event_id();
        let exception = to_exception(error);
        let event = ObservabilityEvent {
            event_id: event_id.clone(),
            timestamp: crate::now_ms(),
            level: Level::Error,
            message: None,
            exception: Some(vec![exception]),
            breadcrumbs: None,
            user: None,
            request: None,
            tags: None,
            contexts: None,
            release: self.state.options.release.clone(),
            environment: self.state.options.environment.clone(),
            sdk: Self::sdk_info(),
        };
        self.dispatch(event, event_id)
    }

    /// Capture an error from any `Display` value (e.g. `anyhow::Error`, a
    /// `String`, a `&str`) when you don't have a `std::error::Error`. Captures a
    /// stack at the call site; no source chain.
    pub fn capture_error_message(
        &self,
        type_name: impl Into<String>,
        message: impl Into<String>,
    ) -> String {
        let event_id = crate::new_event_id();
        let exception = ExceptionInfo {
            r#type: type_name.into(),
            value: crate::pii::scrub_string(&message.into()),
            stacktrace: StackTrace {
                frames: crate::stack::capture_stack(),
            },
            cause: None,
        };
        let event = ObservabilityEvent {
            event_id: event_id.clone(),
            timestamp: crate::now_ms(),
            level: Level::Error,
            message: None,
            exception: Some(vec![exception]),
            breadcrumbs: None,
            user: None,
            request: None,
            tags: None,
            contexts: None,
            release: self.state.options.release.clone(),
            environment: self.state.options.environment.clone(),
            sdk: Self::sdk_info(),
        };
        self.dispatch(event, event_id)
    }

    /// Capture a message at the given level. Returns the generated event id.
    pub fn capture_message(&self, message: impl Into<String>, level: Level) -> String {
        let event_id = crate::new_event_id();
        let event = ObservabilityEvent {
            event_id: event_id.clone(),
            timestamp: crate::now_ms(),
            level,
            message: Some(crate::pii::scrub_string(&message.into())),
            exception: None,
            breadcrumbs: None,
            user: None,
            request: None,
            tags: None,
            contexts: None,
            release: self.state.options.release.clone(),
            environment: self.state.options.environment.clone(),
            sdk: Self::sdk_info(),
        };
        self.dispatch(event, event_id)
    }

    /// Apply scope + before_send, then fire both capture paths.
    fn dispatch(&self, event: ObservabilityEvent, event_id: String) -> String {
        let event = current_scope().apply_to_event(event);
        let event = match &self.state.options.before_send {
            Some(hook) => match hook(event) {
                Some(e) => e,
                None => return event_id, // dropped
            },
            None => event,
        };

        // Capture handler (e.g. OTel span events) — fire first so a slow/failed
        // transport spawn doesn't suppress it.
        if let Some(handler) = &self.state.capture_handler {
            handler(&event);
        }

        // HTTP transport — fire-and-forget so capture never blocks the caller.
        // Only spawn if a Tokio runtime is present; outside one (e.g. a sync
        // unit test), `try_current` returns Err and we drop the event quietly
        // rather than panicking — observability must never take down the host.
        if let Some(transport) = &self.state.transport {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                let t = transport.clone();
                let e = event.clone();
                rt.spawn(async move {
                    t.enqueue(e).await;
                });
            }
        }

        event_id
    }

    /// Flush any queued events. Call on graceful shutdown.
    pub async fn flush(&self) {
        if let Some(transport) = &self.state.transport {
            transport.flush_all().await;
        }
    }
}

/// Convert a `std::error::Error` into an [`ExceptionInfo`], walking the
/// `source()` chain into nested `cause` entries (mirrors the TS `Error.cause`
/// walk). PII-scrubs the message.
fn to_exception<E: std::error::Error + ?Sized>(error: &E) -> ExceptionInfo {
    let type_name = std::any::type_name::<E>();
    let mut exc = ExceptionInfo {
        // type_name on a trait object is unhelpful ("dyn Error"); fall back to
        // a stable label and rely on the message for specifics.
        r#type: short_type_name(type_name),
        value: crate::pii::scrub_string(&error.to_string()),
        stacktrace: StackTrace {
            frames: crate::stack::capture_stack(),
        },
        cause: None,
    };
    if let Some(source) = error.source() {
        exc.cause = Some(Box::new(to_exception_dyn(source)));
    }
    exc
}

fn to_exception_dyn(error: &(dyn std::error::Error + 'static)) -> ExceptionInfo {
    let mut exc = ExceptionInfo {
        r#type: short_type_name(std::any::type_name_of_val(error)),
        value: crate::pii::scrub_string(&error.to_string()),
        // Source frames aren't available; the top-level capture has the stack.
        stacktrace: StackTrace::default(),
        cause: None,
    };
    if let Some(source) = error.source() {
        exc.cause = Some(Box::new(to_exception_dyn(source)));
    }
    exc
}

/// Trim a fully-qualified Rust type name to its last path segment, dropping any
/// generic args, e.g. `core::result::Result<...>` → `Result`. Falls back to the
/// full string when there's nothing to trim.
fn short_type_name(full: &str) -> String {
    let head = full.split('<').next().unwrap_or(full);
    head.rsplit("::").next().unwrap_or(head).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct InnerErr;
    impl fmt::Display for InnerErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "inner cause with Bearer leakedtoken123")
        }
    }
    impl std::error::Error for InnerErr {}

    #[derive(Debug)]
    struct OuterErr(InnerErr);
    impl fmt::Display for OuterErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "outer failure")
        }
    }
    impl std::error::Error for OuterErr {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn short_type_name_trims() {
        assert_eq!(short_type_name("core::result::Result<i32, E>"), "Result");
        assert_eq!(short_type_name("OuterErr"), "OuterErr");
    }

    #[test]
    fn capture_exception_builds_cause_chain_and_scrubs() {
        let err = OuterErr(InnerErr);
        let exc = to_exception(&err);
        assert_eq!(exc.value, "outer failure");
        let cause = exc.cause.expect("should have a cause");
        assert!(
            cause.value.contains("Bearer [redacted]"),
            "PII should be scrubbed: {}",
            cause.value
        );
        assert!(!cause.value.contains("leakedtoken123"));
    }

    #[tokio::test]
    async fn capture_via_handler_only_no_dsn() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = Arc::new(AtomicUsize::new(0));
        let c2 = count.clone();
        let client =
            Client::init(ClientOptions::default()).with_capture_handler(Arc::new(move |_event| {
                c2.fetch_add(1, Ordering::SeqCst);
            }));
        let id = client.capture_message("hello", Level::Info);
        assert!(!id.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn before_send_can_drop() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = Arc::new(AtomicUsize::new(0));
        let c2 = count.clone();
        let opts = ClientOptions {
            before_send: Some(Arc::new(|_e| None)), // drop everything
            ..Default::default()
        };
        let client = Client::init(opts).with_capture_handler(Arc::new(move |_| {
            c2.fetch_add(1, Ordering::SeqCst);
        }));
        client.capture_message("dropped", Level::Info);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "before_send None must drop"
        );
    }

    #[tokio::test]
    async fn capture_message_scrubs_pii() {
        let captured = Arc::new(std::sync::Mutex::new(None));
        let cc = captured.clone();
        let client =
            Client::init(ClientOptions::default()).with_capture_handler(Arc::new(move |e| {
                *cc.lock().unwrap() = e.message.clone();
            }));
        client.capture_message("token=supersecretvalue", Level::Warning);
        let msg = captured.lock().unwrap().clone().unwrap();
        assert!(msg.contains("[redacted]"), "{msg}");
        assert!(!msg.contains("supersecretvalue"));
    }
}
