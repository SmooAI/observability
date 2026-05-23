//! KPI cards — the four "Total Logs / Errors / Error Rate / P95" tiles that
//! sit above the logs table (and will be reused above other views).

use eframe::egui;

pub struct Kpi<'a> {
    pub label: &'a str,
    pub value: String,
    pub color: Option<egui::Color32>,
}

pub fn cards_row(ui: &mut egui::Ui, kpis: &[Kpi<'_>]) {
    ui.horizontal(|ui| {
        let card_width = ui.available_width() / kpis.len().max(1) as f32 - 8.0;
        for kpi in kpis {
            ui.allocate_ui_with_layout(
                egui::vec2(card_width, 64.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(kpi.label)
                                    .small()
                                    .color(egui::Color32::from_gray(150)),
                            );
                            let value = egui::RichText::new(&kpi.value).heading().strong();
                            let value = if let Some(c) = kpi.color {
                                value.color(c)
                            } else {
                                value
                            };
                            ui.label(value);
                        });
                },
            );
        }
    });
}
