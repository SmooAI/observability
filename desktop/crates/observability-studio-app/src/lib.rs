//! Top-level Dioxus app component + bootstrap of global signals + injected
//! stylesheet. Mirrors smooblue's lib.rs shape so the two desktop apps feel
//! like siblings.

use dioxus::prelude::*;

pub mod components;
pub mod persistence;
pub mod state;
pub mod views;

use components::{NavRail, StatusBar};
use observability_studio_theme::{APP_STYLES, BRAND_STYLES};
use state::{ActiveSource, RemoteView};
use views::{ErrorsView, LogsView, MetricsView, SettingsDialog, WelcomeView};

#[component]
pub fn App() -> Element {
    // Install global signals + load persisted org registry + UI state.
    state::use_bootstrap();

    let active_source = use_context::<Signal<ActiveSource>>();
    let active_view = use_context::<Signal<RemoteView>>();
    let settings_open = use_context::<Signal<bool>>();

    // Persist active_source + active_view whenever either changes so the next
    // launch lands where the user left off. `use_effect` re-runs when its
    // dependencies (read via the signal getter) change.
    use_effect(move || {
        state::persist_ui_state(active_source(), active_view());
    });

    rsx! {
        // Two style blocks: shared brand foundation first (so app overrides
        // win on conflict via cascade), then this app's own shell + view CSS.
        style { "{BRAND_STYLES}" }
        style { "{APP_STYLES}" }
        div { class: "shell",
            div { class: "shell__body",
                NavRail {}
                main { class: "shell__main",
                    {render_main(active_source(), active_view())}
                }
            }
            StatusBar {}
        }
        if settings_open() {
            SettingsDialog {}
        }
    }
}

fn render_main(source: ActiveSource, view: RemoteView) -> Element {
    match source {
        ActiveSource::Local => rsx! { WelcomeView {} },
        ActiveSource::Remote(org_id) => match view {
            RemoteView::Logs => rsx! { LogsView { org_id } },
            RemoteView::Errors => rsx! { ErrorsView { org_id } },
            RemoteView::Metrics => rsx! { MetricsView { org_id } },
        },
    }
}
