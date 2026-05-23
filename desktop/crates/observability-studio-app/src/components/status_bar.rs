//! Persistent bottom strip — connection dot, API base, current source label,
//! version. Always rendered.

use dioxus::prelude::*;

use crate::persistence::OrgRegistry;
use crate::state::{current_remote_org, ActiveSource};

#[component]
pub fn StatusBar() -> Element {
    let source = use_context::<Signal<ActiveSource>>();
    let registry = use_context::<Signal<OrgRegistry>>();

    let active_org = current_remote_org(source(), &registry());
    let api_base = active_org
        .as_ref()
        .map(|o| o.base_url.clone())
        .unwrap_or_else(|| "https://api.smoo.ai".to_string());
    let source_label = match source() {
        ActiveSource::Local => "Local logs".to_string(),
        ActiveSource::Remote(_) => active_org
            .as_ref()
            .map(|o| o.label.clone())
            .unwrap_or_else(|| "No org".to_string()),
    };

    rsx! {
        footer { class: "status-bar",
            span { class: "status-bar__dot" }
            span { class: "status-bar__api", "{api_base}" }
            span { class: "status-bar__sep" }
            span { class: "status-bar__source", "{source_label}" }
            span { class: "status-bar__version",
                "smooai-observability-studio · v",
                {env!("CARGO_PKG_VERSION")}
            }
        }
    }
}
