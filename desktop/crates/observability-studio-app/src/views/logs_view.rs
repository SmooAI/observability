use dioxus::prelude::*;
use uuid::Uuid;

use crate::components::icons::FileTextIcon;
use crate::persistence::OrgRegistry;

#[component]
pub fn LogsView(org_id: Uuid) -> Element {
    let registry = use_context::<Signal<OrgRegistry>>();
    let label = registry()
        .entries
        .iter()
        .find(|e| e.org_id == org_id)
        .map(|e| e.label.clone())
        .unwrap_or_default();

    rsx! {
        header { class: "view-header",
            div { class: "view-header__icon", FileTextIcon {} }
            div { class: "view-header__title-block",
                div { class: "view-header__title", "Logs" }
                div { class: "view-header__sub",
                    "{label} — full-text + facets across CloudWatch via /logs/query"
                }
            }
        }
        div { class: "view-body",
            div { class: "view-stub",
                "Logs explorer ships in the next pearl after the Dioxus shell stabilises. The Rust ApiClient already supports POST /logs/query, /logs/facets, and /logs/stats."
            }
        }
    }
}
