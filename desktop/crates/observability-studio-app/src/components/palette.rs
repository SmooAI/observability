//! Cmd/Ctrl+K command palette. Linear/Raycast/Slack-style overlay listing
//! every navigation target in the app — switch source, switch view, open
//! Settings — with a fuzzy substring filter and keyboard-driven selection.
//!
//! Intentionally light: no external matcher crate. The total command set is
//! tiny (handful of orgs + 3 views + Settings) so a plain case-insensitive
//! substring filter is plenty.

use dioxus::prelude::*;
use uuid::Uuid;

use crate::persistence::OrgRegistry;
use crate::state::{persist_ui_state, ActiveSource, PaletteOpen, RemoteView};

#[derive(Clone, Debug, PartialEq)]
pub enum PaletteAction {
    SelectLocal,
    SelectRemote(Uuid),
    SwitchView(RemoteView),
    OpenSettings,
}

#[derive(Clone, Debug)]
struct Command {
    action: PaletteAction,
    /// `"Source · Local"` → first segment is the section, second is the
    /// label. Stays one string so the renderer + filter share one source.
    label: String,
    /// Lowercase searchable form, pre-computed.
    haystack: String,
    /// Optional secondary text (description, shown right-aligned).
    sub: Option<String>,
}

#[component]
pub fn CommandPalette() -> Element {
    let mut palette_open = use_context::<Signal<PaletteOpen>>();
    let mut active_source = use_context::<Signal<ActiveSource>>();
    let mut active_view = use_context::<Signal<RemoteView>>();
    let mut settings_open = use_context::<Signal<bool>>();
    let registry = use_context::<Signal<OrgRegistry>>();

    let mut query = use_signal(String::new);
    let mut selected_idx = use_signal(|| 0usize);

    // Build the command list. Cheap to rebuild per render — the set is tiny.
    let commands = build_commands(&registry().entries);

    // Filter against the current query.
    let needle = query().trim().to_ascii_lowercase();
    let filtered: Vec<(usize, Command)> = commands
        .iter()
        .cloned()
        .enumerate()
        .filter(|(_, c)| needle.is_empty() || c.haystack.contains(&needle))
        .collect();

    // Clamp the selection so a long filter doesn't index off the end.
    if !filtered.is_empty() && selected_idx() >= filtered.len() {
        selected_idx.set(filtered.len() - 1);
    }

    let dispatch = move |action: PaletteAction| {
        match action {
            PaletteAction::SelectLocal => {
                active_source.set(ActiveSource::Local);
            }
            PaletteAction::SelectRemote(id) => {
                active_source.set(ActiveSource::Remote(id));
            }
            PaletteAction::SwitchView(v) => {
                active_view.set(v);
            }
            PaletteAction::OpenSettings => {
                settings_open.set(true);
            }
        }
        // Persist the UI state on every navigation so we don't depend on
        // App's use_effect firing in time for a quick close-then-quit.
        persist_ui_state(active_source(), active_view());
        palette_open.set(PaletteOpen(false));
        query.set(String::new());
        selected_idx.set(0);
    };

    let close = move |_| palette_open.set(PaletteOpen(false));

    // Snapshot the dispatch target up front so the keyboard handler can fire
    // it without borrowing `filtered` (Element renderers can't capture borrows).
    let enter_action: Option<PaletteAction> = filtered
        .get(selected_idx())
        .map(|(_, c)| c.action.clone());

    rsx! {
        div {
            class: "palette__backdrop",
            onclick: close,
            tabindex: -1,
            div {
                class: "palette",
                onclick: move |evt| evt.stop_propagation(),
                input {
                    class: "palette__input",
                    autofocus: true,
                    placeholder: "Jump to anywhere…",
                    value: "{query()}",
                    oninput: move |evt| {
                        query.set(evt.value());
                        selected_idx.set(0);
                    },
                    onkeydown: {
                        let enter_action = enter_action.clone();
                        let filtered_len = filtered.len();
                        let mut dispatch = dispatch;
                        move |evt: KeyboardEvent| match evt.key() {
                            Key::Escape => palette_open.set(PaletteOpen(false)),
                            Key::ArrowDown => {
                                if filtered_len > 0 {
                                    let next = (selected_idx() + 1) % filtered_len;
                                    selected_idx.set(next);
                                }
                            }
                            Key::ArrowUp => {
                                if filtered_len > 0 {
                                    let prev = if selected_idx() == 0 {
                                        filtered_len - 1
                                    } else {
                                        selected_idx() - 1
                                    };
                                    selected_idx.set(prev);
                                }
                            }
                            Key::Enter => {
                                if let Some(action) = enter_action.clone() {
                                    dispatch(action);
                                }
                            }
                            _ => {}
                        }
                    },
                }
                div { class: "palette__list",
                    if filtered.is_empty() {
                        div { class: "palette__empty", "No matching commands." }
                    }
                    for (i, (_, cmd)) in filtered.iter().enumerate() {
                        {
                            let active = i == selected_idx();
                            let class = if active { "palette__row palette__row--active" } else { "palette__row" };
                            let label = cmd.label.clone();
                            let sub = cmd.sub.clone().unwrap_or_default();
                            let action = cmd.action.clone();
                            let mut dispatch = dispatch;
                            rsx! {
                                div {
                                    key: "{label}",
                                    class: "{class}",
                                    onmouseenter: move |_| selected_idx.set(i),
                                    onclick: move |_| dispatch(action.clone()),
                                    span { class: "palette__row-label", "{label}" }
                                    if !sub.is_empty() {
                                        span { class: "palette__row-sub", "{sub}" }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "palette__footer",
                    span { class: "palette__shortcut", "↑ ↓ to navigate · ↵ to select · esc to close" }
                }
            }
        }
    }
}

/// Build the static + registry-derived command list.
fn build_commands(orgs: &[crate::persistence::OrgEntry]) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::with_capacity(8 + orgs.len());

    // Source: Local
    out.push(make("Source · Local", "switch to local logs", PaletteAction::SelectLocal));
    // Source: one per org
    for o in orgs {
        let label = format!("Source · {}", o.label);
        let sub = format!("{} · {}", short_id(&o.org_id), o.base_url);
        out.push(Command {
            action: PaletteAction::SelectRemote(o.org_id),
            haystack: format!("{label} {sub}").to_ascii_lowercase(),
            label,
            sub: Some(sub),
        });
    }
    // Views
    out.push(make("View · Logs", "show the Logs explorer", PaletteAction::SwitchView(RemoteView::Logs)));
    out.push(make("View · Errors", "show the Errors list", PaletteAction::SwitchView(RemoteView::Errors)));
    out.push(make("View · Metrics", "show Metrics + heatmap", PaletteAction::SwitchView(RemoteView::Metrics)));
    out.push(make("Settings", "manage connected orgs", PaletteAction::OpenSettings));
    out
}

fn make(label: &str, sub: &str, action: PaletteAction) -> Command {
    Command {
        action,
        haystack: format!("{label} {sub}").to_ascii_lowercase(),
        label: label.to_string(),
        sub: Some(sub.to_string()),
    }
}

fn short_id(id: &Uuid) -> String {
    let full = id.to_string();
    if full.len() < 13 {
        full
    } else {
        format!("{}…{}", &full[..8], &full[full.len() - 4..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::OrgEntry;

    fn fake_org(label: &str) -> OrgEntry {
        OrgEntry {
            org_id: Uuid::new_v4(),
            label: label.into(),
            base_url: "https://api.smoo.ai".into(),
            client_id_preview: "cid-…".into(),
        }
    }

    #[test]
    fn build_commands_includes_views_and_orgs() {
        let cmds = build_commands(&[fake_org("smoo prod"), fake_org("dev")]);
        // Local + 2 orgs + 3 views + Settings = 7
        assert_eq!(cmds.len(), 7);
        let labels: Vec<&str> = cmds.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"Source · Local"));
        assert!(labels.contains(&"Source · smoo prod"));
        assert!(labels.contains(&"View · Logs"));
        assert!(labels.contains(&"View · Metrics"));
        assert!(labels.contains(&"Settings"));
    }

    #[test]
    fn haystack_is_lowercase_for_fuzzy_filter() {
        let cmds = build_commands(&[fake_org("Smoo Prod")]);
        let prod = cmds.iter().find(|c| c.label.ends_with("Smoo Prod")).unwrap();
        assert!(prod.haystack.contains("smoo prod"));
        assert!(!prod.haystack.chars().any(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn short_id_handles_full_uuid_and_short_input() {
        let full = Uuid::new_v4();
        let s = short_id(&full);
        assert!(s.contains("…"));
        assert!(s.len() <= 16);
    }
}
