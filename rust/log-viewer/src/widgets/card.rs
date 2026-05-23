//! Consistent surface styling for cards, list rows, and side rails. Every
//! "surface" in the app should run through one of these helpers so we get a
//! single place to nudge radius, padding, and stroke as the design evolves.

use eframe::egui::{self, Color32, Frame, Margin, Rounding, Stroke};

/// Standard card — subtle border, generous padding, brand-aware fill that's
/// just a hair brighter than the panel background.
pub fn card(style: &egui::Style) -> Frame {
    let dark = style.visuals.dark_mode;
    let fill = if dark {
        Color32::from_rgb(0x0d, 0x14, 0x24)
    } else {
        Color32::from_rgb(0xfb, 0xfc, 0xfe)
    };
    let stroke_color = if dark {
        Color32::from_rgba_unmultiplied(0xbb, 0xde, 0xf0, 40)
    } else {
        Color32::from_rgba_unmultiplied(0x1a, 0x58, 0x78, 30)
    };
    Frame::none()
        .fill(fill)
        .stroke(Stroke { width: 1.0, color: stroke_color })
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(14.0))
}

/// Pill — small rounded badge for chips and inline labels.
pub fn pill(bg: Color32) -> Frame {
    Frame::none()
        .fill(bg)
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(8.0, 3.0))
}

/// Active-row indicator strip — vertical accent stripe drawn on the left of
/// the row to anchor the eye to the current selection in the nav rail.
pub fn accent_stripe(ui: &mut egui::Ui, color: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, height), egui::Sense::hover());
    ui.painter().rect_filled(rect, Rounding::same(2.0), color);
}
