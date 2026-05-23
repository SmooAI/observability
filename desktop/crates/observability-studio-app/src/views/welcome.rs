//! Empty-state shown when the active source is Local (no remote view to
//! render) — doubles as the first-run "Add your first org" affordance.

use dioxus::prelude::*;

use crate::components::icons::*;
use crate::persistence::OrgRegistry;

#[component]
pub fn WelcomeView() -> Element {
    let registry = use_context::<Signal<OrgRegistry>>();
    let mut settings_open = use_context::<Signal<bool>>();
    let has_orgs = !registry().entries.is_empty();

    rsx! {
        div { class: "welcome",
            div { class: "welcome__card",
                div { class: "welcome__icon",
                    span { style: "width:24px;height:24px;display:inline-flex;",
                        SparkleIcon {}
                    }
                }
                div { class: "welcome__title",
                    if has_orgs { "Pick an org to start" } else { "Welcome to Observability Studio" }
                }
                div { class: "welcome__body",
                    if has_orgs {
                        "Choose one of your connected SmooAI orgs from the sidebar to view its logs, errors, and metrics."
                    } else {
                        "Connect a SmooAI org with an M2M client_id / client_secret pair to see live logs, errors, and metrics from api.smoo.ai."
                    }
                }
                if !has_orgs {
                    button {
                        class: "btn btn--primary btn--lg",
                        onclick: move |_| settings_open.set(true),
                        span { style: "width:16px;height:16px;display:inline-flex;", PlusIcon {} }
                        "Add your first org"
                    }
                }
            }
        }
    }
}
