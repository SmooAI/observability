use dioxus::prelude::*;
use uuid::Uuid;

use crate::components::icons::ActivityIcon;
use crate::persistence::OrgRegistry;

#[component]
pub fn MetricsView(org_id: Uuid) -> Element {
    let registry = use_context::<Signal<OrgRegistry>>();
    let label = registry()
        .entries
        .iter()
        .find(|e| e.org_id == org_id)
        .map(|e| e.label.clone())
        .unwrap_or_default();

    rsx! {
        header { class: "view-header",
            div { class: "view-header__icon", ActivityIcon {} }
            div { class: "view-header__title-block",
                div { class: "view-header__title", "Metrics" }
                div { class: "view-header__sub",
                    "{label} — counters, gauges, histograms + latency heatmap via /metrics"
                }
            }
        }
        div { class: "view-body",
            div { class: "view-stub",
                "Metrics + heatmap port ships next."
            }
        }
    }
}
