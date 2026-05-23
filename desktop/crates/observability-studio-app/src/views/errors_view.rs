use dioxus::prelude::*;
use uuid::Uuid;

use crate::components::icons::AlertTriangleIcon;
use crate::persistence::OrgRegistry;

#[component]
pub fn ErrorsView(org_id: Uuid) -> Element {
    let registry = use_context::<Signal<OrgRegistry>>();
    let label = registry()
        .entries
        .iter()
        .find(|e| e.org_id == org_id)
        .map(|e| e.label.clone())
        .unwrap_or_default();

    rsx! {
        header { class: "view-header",
            div { class: "view-header__icon", AlertTriangleIcon {} }
            div { class: "view-header__title-block",
                div { class: "view-header__title", "Errors" }
                div { class: "view-header__sub",
                    "{label} — grouped exceptions + stack traces via /errors"
                }
            }
        }
        div { class: "view-body",
            div { class: "view-stub",
                "Errors list + detail (stack-frame viewer, mark-resolved/muted) ships next."
            }
        }
    }
}
