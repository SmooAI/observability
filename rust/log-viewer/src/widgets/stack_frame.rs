//! Renders the `frames[]` array carried in error events' `exception[i].stacktrace.frames`.
//!
//! Frame shape (from OTel / Sentry SDKs):
//!
//! ```json
//! { "filename": "src/foo.ts", "function": "doIt", "lineno": 42,
//!   "abs_path": "/repo/src/foo.ts", "context_line": "throw new Error()",
//!   "pre_context": [...], "post_context": [...] }
//! ```
//!
//! We don't require all fields. Missing keys are simply skipped.

use eframe::egui;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Frame {
    pub filename: Option<String>,
    pub function: Option<String>,
    pub lineno: Option<i64>,
    pub colno: Option<i64>,
    pub abs_path: Option<String>,
    pub module: Option<String>,
    pub context_line: Option<String>,
    #[serde(default)]
    pub pre_context: Vec<String>,
    #[serde(default)]
    pub post_context: Vec<String>,
}

pub fn render_frames(ui: &mut egui::Ui, frames: &[Frame]) {
    for (i, frame) in frames.iter().enumerate() {
        let function = frame.function.as_deref().unwrap_or("<anonymous>");
        let file = frame.filename.as_deref().or(frame.module.as_deref()).unwrap_or("<unknown>");
        let line = frame.lineno.map(|n| n.to_string()).unwrap_or_default();

        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                let header = if line.is_empty() {
                    format!("at {function} ({file})")
                } else {
                    format!("at {function} ({file}:{line})")
                };
                ui.label(egui::RichText::new(header).monospace().small().strong());

                if frame.context_line.is_some()
                    || !frame.pre_context.is_empty()
                    || !frame.post_context.is_empty()
                {
                    ui.add_space(4.0);
                    render_source_context(ui, frame, i);
                }
            });
    }
}

fn render_source_context(ui: &mut egui::Ui, frame: &Frame, frame_idx: usize) {
    let base_line = frame.lineno.unwrap_or(0);
    let pre_offset = frame.pre_context.len() as i64;
    let start_line = base_line - pre_offset;

    egui::ScrollArea::vertical()
        .max_height(120.0)
        .id_source(format!("stackframe-ctx-{frame_idx}"))
        .show(ui, |ui| {
            let mut line_no = start_line;
            for line in &frame.pre_context {
                source_line(ui, line_no, line, false);
                line_no += 1;
            }
            if let Some(ctx) = &frame.context_line {
                source_line(ui, line_no, ctx, true);
                line_no += 1;
            }
            for line in &frame.post_context {
                source_line(ui, line_no, line, false);
                line_no += 1;
            }
        });
}

fn source_line(ui: &mut egui::Ui, line_no: i64, content: &str, hit: bool) {
    let prefix = format!("{line_no:>4} ");
    let mut text = egui::RichText::new(format!("{prefix}{content}")).monospace().small();
    if hit {
        text = text.background_color(egui::Color32::from_rgb(60, 30, 30)).strong();
    } else {
        text = text.color(egui::Color32::from_gray(160));
    }
    ui.label(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frame_parses_minimal_shape() {
        let f: Frame = serde_json::from_value(json!({
            "function": "doIt",
            "filename": "foo.ts",
            "lineno": 12
        })).unwrap();
        assert_eq!(f.function.as_deref(), Some("doIt"));
        assert_eq!(f.lineno, Some(12));
        assert!(f.pre_context.is_empty());
    }

    #[test]
    fn frame_parses_with_source_context() {
        let f: Frame = serde_json::from_value(json!({
            "function": "doIt",
            "filename": "foo.ts",
            "lineno": 12,
            "context_line": "throw new Error()",
            "pre_context": ["// before"],
            "post_context": ["// after"]
        })).unwrap();
        assert_eq!(f.pre_context.len(), 1);
        assert_eq!(f.post_context.len(), 1);
        assert!(f.context_line.is_some());
    }
}
