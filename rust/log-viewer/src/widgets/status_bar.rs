//! Persistent footer strip — shows the connection target, current source
//! label, last refresh time, and any global error/notice. Always rendered,
//! anchored to the bottom of the window.

use eframe::egui::{self, Color32, Layout};

pub struct StatusBar<'a> {
    pub api_base: &'a str,
    pub source_label: &'a str,
    pub last_refresh: Option<&'a str>,
    pub error: Option<&'a str>,
}

impl<'a> StatusBar<'a> {
    pub fn ui(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Left: connection target + active source.
            let dot_color = if self.error.is_some() {
                crate::theme::smoo::RED
            } else {
                crate::theme::smoo::GREEN
            };
            ui.label(egui::RichText::new("●").color(dot_color));
            ui.label(
                egui::RichText::new(self.api_base)
                    .small()
                    .color(Color32::from_gray(160)),
            );
            ui.separator();
            ui.label(
                egui::RichText::new(self.source_label)
                    .small()
                    .color(Color32::from_gray(200))
                    .strong(),
            );

            // Right: last refresh + error tail.
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(err) = self.error {
                    ui.label(
                        egui::RichText::new(format!("⚠ {err}"))
                            .small()
                            .color(crate::theme::smoo::RED),
                    );
                }
                if let Some(when) = self.last_refresh {
                    ui.label(
                        egui::RichText::new(format!("updated {when}"))
                            .small()
                            .color(Color32::from_gray(150)),
                    );
                }
            });
        });
    }
}
