//! Global signals + the bootstrap hook that installs them on the App
//! component. Mirrors smooblue's `state::use_bootstrap`.
//!
//! We deliberately keep state flat — three signals, accessed by descendants
//! via `use_context::<Signal<T>>()`. No reducer pattern, no global store.

use std::sync::Arc;

use dioxus::prelude::*;
use observability_studio_client::AuthManager;
use uuid::Uuid;

use crate::persistence::{self, OrgEntry, OrgRegistry};

/// Which data source the user is currently viewing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveSource {
    Local,
    Remote(Uuid),
}

impl Default for ActiveSource {
    fn default() -> Self {
        Self::Local
    }
}

/// Which dashboard view is showing inside a remote source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RemoteView {
    #[default]
    Logs,
    Errors,
    Metrics,
}

/// Shared HTTP client + AuthManager — handed to descendants via context so
/// each view doesn't have to rebuild them.
pub struct ApiState {
    pub http: reqwest::Client,
    pub auth: AuthManager,
}

impl ApiState {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "smooai-observability-studio/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .expect("reqwest::Client should build");
        let auth = AuthManager::new(http.clone());
        Self { http, auth }
    }
}

/// Install the three top-level signals + the shared API state. Call from the
/// App component root.
pub fn use_bootstrap() {
    use_context_provider::<Signal<ActiveSource>>(|| {
        Signal::new(ActiveSource::default())
    });
    use_context_provider::<Signal<RemoteView>>(|| Signal::new(RemoteView::default()));
    use_context_provider::<Signal<bool>>(|| Signal::new(false));
    use_context_provider::<Signal<OrgRegistry>>(|| {
        Signal::new(persistence::OrgRegistry::load_or_default())
    });
    use_context_provider::<Arc<ApiState>>(|| Arc::new(ApiState::new()));
}

/// Convenience accessor used across views to find the entry for the current
/// remote source (if any).
pub fn current_remote_org(
    source: ActiveSource,
    registry: &OrgRegistry,
) -> Option<OrgEntry> {
    if let ActiveSource::Remote(id) = source {
        registry.entries.iter().find(|e| e.org_id == id).cloned()
    } else {
        None
    }
}
