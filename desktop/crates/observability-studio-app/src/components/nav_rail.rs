//! Left navigation rail — sources, views, settings. Owns both source +
//! view selection; mirrors the smooblue sidebar in shape.

use dioxus::prelude::*;

use crate::components::icons::*;
use crate::persistence::OrgRegistry;
use crate::state::{ActiveSource, RemoteView};

#[component]
pub fn NavRail() -> Element {
    let mut source = use_context::<Signal<ActiveSource>>();
    let mut view = use_context::<Signal<RemoteView>>();
    let mut settings_open = use_context::<Signal<bool>>();
    let registry = use_context::<Signal<OrgRegistry>>();

    let is_remote = matches!(source(), ActiveSource::Remote(_));

    rsx! {
        aside { class: "rail",
            div { class: "rail__brand",
                // Brand mark — `.brand-badge` (gradient pill backdrop) +
                // smoo monogram (SVG with currentColor) both come from the
                // shared `@smooai/ui` crate. Apps don't redraw this.
                div {
                    class: "brand-badge rail__brand-mark",
                    style: "width:30px;height:30px;color:white;",
                    dangerous_inner_html: observability_studio_theme::MONOGRAM_SVG,
                }
                div {
                    div { class: "rail__brand-title", "SmooAI" }
                    div { class: "rail__brand-sub", "Observability Studio" }
                }
            }

            div { class: "rail__section",
                div { class: "rail__section-label", "Sources" }
                NavItem {
                    icon: rsx! { HardDriveIcon {} },
                    label: "Local",
                    active: matches!(source(), ActiveSource::Local),
                    on_click: move |_| source.set(ActiveSource::Local),
                }
                for entry in registry().entries.iter().cloned() {
                    NavItem {
                        key: "{entry.org_id}",
                        icon: rsx! { CloudIcon {} },
                        label: entry.label.clone(),
                        active: matches!(source(), ActiveSource::Remote(o) if o == entry.org_id),
                        on_click: move |_| source.set(ActiveSource::Remote(entry.org_id)),
                    }
                }
            }

            if is_remote {
                div { class: "rail__section",
                    div { class: "rail__section-label", "Views" }
                    NavItem {
                        icon: rsx! { FileTextIcon {} },
                        label: "Logs",
                        active: view() == RemoteView::Logs,
                        on_click: move |_| view.set(RemoteView::Logs),
                    }
                    NavItem {
                        icon: rsx! { AlertTriangleIcon {} },
                        label: "Errors",
                        active: view() == RemoteView::Errors,
                        on_click: move |_| view.set(RemoteView::Errors),
                    }
                    NavItem {
                        icon: rsx! { ActivityIcon {} },
                        label: "Metrics",
                        active: view() == RemoteView::Metrics,
                        on_click: move |_| view.set(RemoteView::Metrics),
                    }
                }
            }

            div { class: "rail__spacer" }
            NavItem {
                icon: rsx! { SettingsIcon {} },
                label: "Settings",
                active: settings_open(),
                on_click: move |_| settings_open.set(!settings_open()),
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct NavItemProps {
    icon: Element,
    label: String,
    active: bool,
    on_click: EventHandler<MouseEvent>,
}

#[component]
fn NavItem(props: NavItemProps) -> Element {
    let class = if props.active {
        "rail__item rail__item--active"
    } else {
        "rail__item"
    };
    rsx! {
        button {
            class: "{class}",
            onclick: move |evt| props.on_click.call(evt),
            span { class: "rail__item-icon", {props.icon} }
            span { class: "rail__item-label", "{props.label}" }
        }
    }
}
