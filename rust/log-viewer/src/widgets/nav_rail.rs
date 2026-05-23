//! Left navigation rail — replaces the previous horizontal source + view
//! tab strips with a single vertical sidebar. Two "sections":
//!
//! - **Sources**: 💾 Local + ☁ org chips
//! - **Views** (only shown when a remote source is active): 📜 Logs, ⚠ Errors,
//!   📊 Metrics
//!
//! All targets are large click-targets with an accent stripe on the active
//! row.

use eframe::egui::{self, Align, Color32, Layout, Margin, Sense};

use crate::widgets::card;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavTarget {
    SourceLocal,
    SourceRemote(uuid::Uuid),
    ViewLogs,
    ViewErrors,
    ViewMetrics,
    OpenSettings,
}

pub struct NavItem<'a> {
    pub target: NavTarget,
    pub icon: &'a str,
    pub label: &'a str,
    pub active: bool,
}

/// Renders a single tappable item. Returns the click response so callers can
/// decide what to do.
pub fn item(ui: &mut egui::Ui, accent: Color32, it: &NavItem<'_>) -> egui::Response {
    let frame = if it.active {
        card::card(ui.style())
            .fill(crate::theme::lerp(
                ui.visuals().panel_fill,
                accent,
                0.18,
            ))
            .inner_margin(Margin {
                left: 10.0,
                right: 12.0,
                top: 8.0,
                bottom: 8.0,
            })
    } else {
        card::card(ui.style())
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .inner_margin(Margin {
                left: 10.0,
                right: 12.0,
                top: 8.0,
                bottom: 8.0,
            })
    };

    let inner = frame
        .show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                if it.active {
                    card::accent_stripe(ui, accent, 18.0);
                    ui.add_space(6.0);
                } else {
                    // reserve the same horizontal space so labels line up
                    ui.add_space(3.0 + 6.0);
                }
                ui.label(
                    egui::RichText::new(it.icon)
                        .size(16.0)
                        .color(if it.active {
                            accent
                        } else {
                            ui.visuals().text_color()
                        }),
                );
                ui.add_space(8.0);
                let mut label = egui::RichText::new(it.label);
                if it.active {
                    label = label.strong();
                }
                ui.label(label);
            });
        })
        .response;

    inner.interact(Sense::click())
}

/// Section header — small uppercase label above a group of nav items.
pub fn section_header(ui: &mut egui::Ui, label: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(label.to_ascii_uppercase())
            .small()
            .color(Color32::from_gray(140))
            .strong(),
    );
    ui.add_space(2.0);
}
