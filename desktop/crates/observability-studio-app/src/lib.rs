//! Top-level Dioxus app component + bootstrap of global signals + injected
//! stylesheet. Mirrors smooblue's lib.rs shape so the two desktop apps feel
//! like siblings.

use dioxus::prelude::*;

pub mod components;
pub mod persistence;
pub mod state;
pub mod views;

use components::{CommandPalette, NavRail, StatusBar};
use observability_studio_theme::{APP_STYLES, BRAND_STYLES};
use state::{ActiveSource, PaletteOpen, RemoteView};
use views::{ErrorsView, LogsView, MetricsView, SettingsDialog, WelcomeView};

#[component]
pub fn App() -> Element {
    // Install global signals + load persisted org registry + UI state.
    state::use_bootstrap();

    let active_source = use_context::<Signal<ActiveSource>>();
    let active_view = use_context::<Signal<RemoteView>>();
    let settings_open = use_context::<Signal<bool>>();
    let mut palette_open = use_context::<Signal<PaletteOpen>>();

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
        div {
            class: "shell",
            tabindex: 0,
            // Global Cmd/Ctrl+K — toggle the palette. Bound on the root
            // (focusable via `tabindex: 0`) so it works regardless of which
            // panel currently has focus.
            onkeydown: move |evt: KeyboardEvent| {
                if evt.key() == Key::Character("k".to_string())
                    && (evt.modifiers().meta() || evt.modifiers().ctrl())
                {
                    let cur = palette_open().0;
                    palette_open.set(PaletteOpen(!cur));
                    evt.prevent_default();
                }
            },
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
        if palette_open().0 {
            CommandPalette {}
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
