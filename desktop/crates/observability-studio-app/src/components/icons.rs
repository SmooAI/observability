//! Inline SVG icons. Matches smooblue's "no icon library" convention —
//! hand-coded Lucide-style strokes scoped to the `.icon` class so the parent
//! text-color/size flow through.

use dioxus::prelude::*;

/// Render the given Lucide path on a 24×24 stroked viewBox.
#[component]
pub fn Icon(path: &'static str) -> Element {
    rsx! {
        svg {
            class: "icon",
            view_box: "0 0 24 24",
            xmlns: "http://www.w3.org/2000/svg",
            path { d: "{path}" }
        }
    }
}

// Per-icon convenience components so call sites read like `HardDriveIcon {}`
// rather than passing strings around.

#[component]
pub fn HardDriveIcon() -> Element {
    // Lucide: hard-drive
    rsx! { Icon { path: "M22 12H2 M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11Z M6 16h.01 M10 16h.01" }}
}

#[component]
pub fn CloudIcon() -> Element {
    rsx! { Icon { path: "M17.5 19a4.5 4.5 0 1 0-1.5-8.74A6 6 0 0 0 6.5 9a5 5 0 0 0-1.45 9.79 M17.5 19H6.5" }}
}

#[component]
pub fn FileTextIcon() -> Element {
    rsx! { Icon { path: "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z M14 2v4a2 2 0 0 0 2 2h4 M16 13H8 M16 17H8 M10 9H8" }}
}

#[component]
pub fn AlertTriangleIcon() -> Element {
    rsx! { Icon { path: "M21.73 18 13.73 4a2 2 0 0 0-3.46 0L2.27 18a2 2 0 0 0 1.73 3h16a2 2 0 0 0 1.73-3Z M12 9v4 M12 17h.01" }}
}

#[component]
pub fn ActivityIcon() -> Element {
    rsx! { Icon { path: "M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.5.5 0 0 1-.96 0L9.24 2.18a.5.5 0 0 0-.96 0l-2.35 8.36A2 2 0 0 1 4 12H2" }}
}

#[component]
pub fn SettingsIcon() -> Element {
    rsx! { Icon { path: "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2Z M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" }}
}

#[component]
pub fn SparkleIcon() -> Element {
    rsx! { Icon { path: "M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z M20 3v4 M22 5h-4 M4 17v2 M5 18H3" }}
}

#[component]
pub fn PlusIcon() -> Element {
    rsx! { Icon { path: "M5 12h14 M12 5v14" }}
}

#[component]
pub fn TrashIcon() -> Element {
    rsx! { Icon { path: "M3 6h18 M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6 M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }}
}

#[component]
pub fn XIcon() -> Element {
    rsx! { Icon { path: "M18 6 6 18 M6 6l12 12" }}
}
