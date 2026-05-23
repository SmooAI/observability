//! Global signals + the bootstrap hook that installs them on the App
//! component. Mirrors smooblue's `state::use_bootstrap`.
//!
//! We deliberately keep state flat — three signals, accessed by descendants
//! via `use_context::<Signal<T>>()`. No reducer pattern, no global store.

use std::sync::Arc;

use dioxus::prelude::*;
use observability_studio_client::AuthManager;
use uuid::Uuid;

use crate::persistence::{self, OrgEntry, OrgRegistry, PersistedSource, PersistedView, UiState};

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

/// Install the top-level signals + the shared API state. Call from the App
/// component root. Both `active_source` and `active_view` rehydrate from
/// `<config_dir>/ui-state.json` so the user lands where they left off.
pub fn use_bootstrap() {
    let registry = persistence::OrgRegistry::load_or_default();
    let saved = UiState::load_or_default();

    // Rehydrate active_source — but if the persisted remote org no longer
    // exists in the registry (user removed it offline), gracefully fall back
    // to Local rather than rendering a stuck "unknown org" header.
    let initial_source = match saved.active_source {
        PersistedSource::Local => ActiveSource::Local,
        PersistedSource::Remote { org_id } => {
            if registry.entries.iter().any(|e| e.org_id == org_id) {
                ActiveSource::Remote(org_id)
            } else {
                ActiveSource::Local
            }
        }
    };
    let initial_view = match saved.active_view {
        PersistedView::Logs => RemoteView::Logs,
        PersistedView::Errors => RemoteView::Errors,
        PersistedView::Metrics => RemoteView::Metrics,
    };

    use_context_provider::<Signal<ActiveSource>>(move || Signal::new(initial_source));
    use_context_provider::<Signal<RemoteView>>(move || Signal::new(initial_view));
    use_context_provider::<Signal<bool>>(|| Signal::new(false));
    use_context_provider::<Signal<OrgRegistry>>(move || Signal::new(registry));
    use_context_provider::<Arc<ApiState>>(|| Arc::new(ApiState::new()));
}

/// Snapshot the current source + view to disk. Called from the App's
/// `use_effect` when either signal changes.
pub fn persist_ui_state(source: ActiveSource, view: RemoteView) {
    let state = UiState {
        active_source: match source {
            ActiveSource::Local => PersistedSource::Local,
            ActiveSource::Remote(org_id) => PersistedSource::Remote { org_id },
        },
        active_view: match view {
            RemoteView::Logs => PersistedView::Logs,
            RemoteView::Errors => PersistedView::Errors,
            RemoteView::Metrics => PersistedView::Metrics,
        },
    };
    state.save();
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
