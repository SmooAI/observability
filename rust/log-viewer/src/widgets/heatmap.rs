//! Latency heatmap widget — mirrors the CSS-grid heatmap in
//! `apps/web/components/observability/metrics/metrics-explorer.tsx`.
//!
//! Renders an (X = time bucket) × (Y = histogram bucket) grid of rectangles
//! where each cell's lightness encodes log-scaled count. Y axis runs from `0`
//! at the bottom to `+∞` at the top so heavy tails appear high in the chart,
//! matching the web layout.
//!
//! Hover any cell to see the precise (time range, bucket, count) tuple.

use eframe::egui;

#[derive(Clone, Debug)]
pub struct HeatmapPoint {
    pub bucket_ms: i64,
    /// `counts.len()` = `bounds.len() + 1` (the extra slot is the `+∞`
    /// overflow). All `HeatmapPoint`s for the same metric share the same
    /// `bounds` shape; we don't enforce that here.
    pub counts: Vec<u64>,
    pub bounds: Vec<f64>,
}

pub struct Heatmap<'a> {
    pub points: &'a [HeatmapPoint],
    pub desired_height: f32,
}

impl<'a> Heatmap<'a> {
    pub fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let bounds = self
            .points
            .iter()
            .find(|p| !p.bounds.is_empty())
            .map(|p| p.bounds.clone())
            .unwrap_or_default();

        // Number of Y rows = histogram-bucket count (incl. overflow).
        let row_count = if bounds.is_empty() {
            // No data — render empty placeholder.
            return ui.add_sized(
                egui::vec2(ui.available_width(), self.desired_height),
                egui::Label::new(
                    egui::RichText::new("no histogram data")
                        .italics()
                        .color(egui::Color32::from_gray(150)),
                ),
            );
        } else {
            bounds.len() + 1
        };
        let col_count = self.points.len().max(1);

        let max_count: u64 = self
            .points
            .iter()
            .flat_map(|p| p.counts.iter().copied())
            .max()
            .unwrap_or(0);

        // Reserve drawing area.
        let available_w = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(available_w, self.desired_height),
            egui::Sense::hover(),
        );

        // Layout — 60px on the left for Y labels, 16px on the bottom for X
        // labels.
        let y_label_w = 60.0;
        let x_label_h = 16.0;
        let plot_left = rect.left() + y_label_w;
        let plot_top = rect.top();
        let plot_right = rect.right();
        let plot_bottom = rect.bottom() - x_label_h;
        let plot_w = (plot_right - plot_left).max(1.0);
        let plot_h = (plot_bottom - plot_top).max(1.0);
        let cell_w = plot_w / col_count as f32;
        let cell_h = plot_h / row_count as f32;

        let painter = ui.painter_at(rect);

        // Background.
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(plot_left, plot_top),
                egui::pos2(plot_right, plot_bottom),
            ),
            0.0,
            egui::Color32::from_gray(20),
        );

        // Cells.
        for (col, point) in self.points.iter().enumerate() {
            for (row_idx, &count) in point.counts.iter().enumerate() {
                // Bottom-up Y: row 0 is the lowest latency bucket.
                let y_top = plot_bottom - (row_idx as f32 + 1.0) * cell_h;
                let x_left = plot_left + col as f32 * cell_w;
                let r = egui::Rect::from_min_size(
                    egui::pos2(x_left, y_top),
                    egui::vec2(cell_w, cell_h),
                );
                let color = log_scaled_color(count, max_count);
                painter.rect_filled(r, 0.0, color);
            }
        }

        // Y-axis labels: bottom-up, sparse (every ~3 rows).
        let stride = (row_count / 6).max(1);
        for row_idx in (0..row_count).step_by(stride) {
            let label = if row_idx == bounds.len() {
                "+∞".to_string()
            } else {
                format_bound(bounds[row_idx])
            };
            let y_top = plot_bottom - (row_idx as f32 + 1.0) * cell_h;
            painter.text(
                egui::pos2(rect.left() + 4.0, y_top + cell_h / 2.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::monospace(10.0),
                egui::Color32::from_gray(160),
            );
        }

        // X-axis labels: first + last + middle.
        if !self.points.is_empty() {
            let first = self.points.first().unwrap().bucket_ms;
            let last = self.points.last().unwrap().bucket_ms;
            painter.text(
                egui::pos2(plot_left, plot_bottom + 4.0),
                egui::Align2::LEFT_TOP,
                format_bucket_ms(first),
                egui::FontId::monospace(10.0),
                egui::Color32::from_gray(160),
            );
            painter.text(
                egui::pos2(plot_right, plot_bottom + 4.0),
                egui::Align2::RIGHT_TOP,
                format_bucket_ms(last),
                egui::FontId::monospace(10.0),
                egui::Color32::from_gray(160),
            );
        }

        // Tooltip on hover.
        if let Some(pos) = response.hover_pos() {
            if pos.x >= plot_left && pos.x <= plot_right && pos.y >= plot_top && pos.y <= plot_bottom
            {
                let col = ((pos.x - plot_left) / cell_w).clamp(0.0, (col_count - 1) as f32) as usize;
                let row_from_bottom =
                    ((plot_bottom - pos.y) / cell_h).clamp(0.0, (row_count - 1) as f32) as usize;
                if let Some(point) = self.points.get(col) {
                    if let Some(&count) = point.counts.get(row_from_bottom) {
                        let bucket_label = if row_from_bottom == bounds.len() {
                            "≥ +∞".to_string()
                        } else if row_from_bottom == 0 {
                            format!("< {}", format_bound(bounds[0]))
                        } else {
                            let lo = bounds.get(row_from_bottom - 1).copied().unwrap_or(0.0);
                            let hi = bounds.get(row_from_bottom).copied().unwrap_or(lo);
                            format!("{} – {}", format_bound(lo), format_bound(hi))
                        };
                        response.clone().on_hover_text(format!(
                            "{}\n{}: {} samples",
                            format_bucket_ms(point.bucket_ms),
                            bucket_label,
                            count
                        ));
                    }
                }
            }
        }

        response
    }
}

/// Cell color: empty cells render at the same lightness as the background (so
/// the heatmap blends into its surroundings), heavy cells render with the
/// Smoo brand blue at high saturation. Log-scaled so single-sample outliers
/// remain visible alongside high-volume buckets.
fn log_scaled_color(count: u64, max_count: u64) -> egui::Color32 {
    if count == 0 || max_count == 0 {
        return egui::Color32::from_gray(20);
    }
    let intensity =
        ((count as f64).ln_1p() / (max_count as f64).ln_1p()).clamp(0.0, 1.0) as f32;
    // Lerp from gray-20 → smoo BLUE_400 → smoo GREEN at the top end so heavy
    // hotspots pop.
    let base = egui::Color32::from_gray(30);
    let mid = crate::theme::smoo::BLUE_400;
    let top = crate::theme::smoo::GREEN;
    if intensity < 0.5 {
        lerp(base, mid, intensity * 2.0)
    } else {
        lerp(mid, top, (intensity - 0.5) * 2.0)
    }
}

fn lerp(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let to = |c: egui::Color32| (c.r() as f32, c.g() as f32, c.b() as f32);
    let (ar, ag, ab) = to(a);
    let (br, bg, bb) = to(b);
    egui::Color32::from_rgb(
        (ar + (br - ar) * t) as u8,
        (ag + (bg - ag) * t) as u8,
        (ab + (bb - ab) * t) as u8,
    )
}

fn format_bound(b: f64) -> String {
    if b >= 1000.0 {
        format!("{:.1}k", b / 1000.0)
    } else if b >= 1.0 {
        format!("{b:.0}")
    } else {
        format!("{b:.3}")
    }
}

fn format_bucket_ms(ms: i64) -> String {
    // Cheap epoch-ms → HH:MM formatter. Avoids pulling chrono into the hot
    // path; we just need axis labels.
    let secs = ms / 1000;
    let mins = (secs / 60) % 60;
    let hours = (secs / 3600) % 24;
    format!("{hours:02}:{mins:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_produces_gray_color() {
        assert_eq!(log_scaled_color(0, 0), egui::Color32::from_gray(20));
        assert_eq!(log_scaled_color(0, 10), egui::Color32::from_gray(20));
    }

    #[test]
    fn max_input_lands_near_top_color() {
        let max = log_scaled_color(1000, 1000);
        let zero = log_scaled_color(0, 1000);
        // Max should be visibly distinct from the empty cell color. Brand
        // GREEN is teal (0x00a6a6) so we can't assert "more green than blue";
        // we assert "brighter overall and not the gray base".
        let total = |c: egui::Color32| c.r() as u16 + c.g() as u16 + c.b() as u16;
        assert!(total(max) > total(zero) + 80, "max should be much brighter than empty");
    }

    #[test]
    fn bound_formatting() {
        assert_eq!(format_bound(0.005), "0.005");
        assert_eq!(format_bound(42.0), "42");
        assert_eq!(format_bound(1500.0), "1.5k");
    }
}
