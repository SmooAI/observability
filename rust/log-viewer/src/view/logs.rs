//! Remote Logs view — mirrors `apps/web/components/observability/logs-explorer.tsx`.
//!
//! State machine:
//!
//! ```text
//! Idle ──refresh──▶ Loading ──response──▶ Loaded
//!                       └────error─────▶ Failed
//! ```
//!
//! Async work runs on the shared tokio runtime; results are funnelled back via
//! a `std::sync::mpsc` channel and drained each frame. We re-request when the
//! user changes the time range, search text, or active source.

use std::sync::mpsc::{Receiver, Sender};

use chrono::{DateTime, Utc};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use uuid::Uuid;

use crate::api::logs::{LogEntry, LogQuery, LogQueryResult, LogStats, TimeRange};
use crate::api::{ApiClient, ApiError};
use crate::widgets::{
    kpi_card::{cards_row, Kpi},
    time_range::{preset_picker, TimePreset},
};

const PAGE_SIZE: u32 = 100;

#[derive(Default)]
pub struct RemoteLogsView {
    pub org_id: Option<Uuid>,
    pub search: String,
    pub preset: TimePreset,
    state: State,
    stats: Option<LogStats>,
    last_error: Option<String>,
    rx: Option<Receiver<FetchOutcome>>,
    /// Set of `LogEntry.timestamp + log_stream` keys currently expanded.
    expanded: std::collections::HashSet<String>,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Loading,
    Loaded(LoadedState),
    Failed,
}

struct LoadedState {
    entries: Vec<LogEntry>,
    total: u64,
    queried_at: DateTime<Utc>,
}

enum FetchOutcome {
    Ok {
        result: LogQueryResult,
        stats: Option<LogStats>,
    },
    Err(String),
}

impl RemoteLogsView {
    pub fn for_org(org_id: Uuid) -> Self {
        Self {
            org_id: Some(org_id),
            ..Default::default()
        }
    }

    /// Render the view inside a CentralPanel. The caller passes the shared
    /// `ApiClient` + tokio handle so we can fire requests without owning them.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        let Some(org_id) = self.org_id else {
            ui.centered_and_justified(|ui| {
                ui.label("No org selected. Add an org under ⚙ Settings.");
            });
            return;
        };

        self.drain_pending(ui.ctx());

        // -- Header: filters + refresh --
        ui.horizontal(|ui| {
            ui.label("Time:");
            let preset_changed = preset_picker(ui, &mut self.preset);
            ui.separator();
            let search_resp = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("search across log fields")
                    .desired_width(280.0),
            );
            let refresh_clicked = ui.button("⟳ Refresh").clicked();
            let should_fetch = preset_changed
                || refresh_clicked
                || search_resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if should_fetch {
                self.fire(org_id, api, runtime);
            }
            if matches!(self.state, State::Loading) {
                ui.spinner();
            }
            if let Some(loaded) = self.loaded_ref() {
                ui.label(
                    egui::RichText::new(format!(
                        "{} rows · queried {}",
                        loaded.entries.len(),
                        loaded
                            .queried_at
                            .format("%H:%M:%S")
                    ))
                    .small()
                    .color(egui::Color32::from_gray(150)),
                );
            }
        });

        // Auto-fire on first frame for this org so the user doesn't have to
        // click Refresh just to see something.
        if matches!(self.state, State::Idle) {
            self.fire(org_id, api, runtime);
        }

        ui.add_space(6.0);
        self.render_kpis(ui);
        ui.add_space(6.0);

        if let Some(err) = &self.last_error {
            ui.colored_label(egui::Color32::from_rgb(220, 100, 100), err);
            ui.add_space(6.0);
        }

        // Snapshot the current entries to avoid borrowing `self` while we render
        // (the table builder needs `&mut ui`).
        if let Some(loaded) = self.loaded_ref() {
            let entries = loaded.entries.clone();
            self.render_table(ui, &entries);
        } else if matches!(self.state, State::Loading) {
            ui.centered_and_justified(|ui| ui.label("Loading…"));
        } else if matches!(self.state, State::Failed) {
            ui.centered_and_justified(|ui| ui.label("Failed — see error above."));
        }
    }

    fn render_kpis(&self, ui: &mut egui::Ui) {
        if let Some(stats) = &self.stats {
            let total = stats.total_logs;
            let errors: u64 = stats
                .logs_by_level
                .iter()
                .filter(|l| matches!(l.level.to_ascii_uppercase().as_str(), "ERROR" | "FATAL"))
                .map(|l| l.count)
                .sum();
            let rate = if total == 0 {
                0.0
            } else {
                errors as f64 * 100.0 / total as f64
            };
            cards_row(
                ui,
                &[
                    Kpi {
                        label: "Total Logs",
                        value: thousands(total),
                        color: None,
                    },
                    Kpi {
                        label: "Errors",
                        value: thousands(errors),
                        color: Some(crate::theme::smoo::RED),
                    },
                    Kpi {
                        label: "Error Rate",
                        value: format!("{rate:.2}%"),
                        color: if rate > 1.0 {
                            Some(crate::theme::smoo::ORANGE)
                        } else {
                            None
                        },
                    },
                    Kpi {
                        label: "P95 Duration",
                        value: format!("{:.0} ms", stats.duration_percentiles.p95),
                        color: None,
                    },
                ],
            );
        }
    }

    fn render_table(&mut self, ui: &mut egui::Ui, entries: &[LogEntry]) {
        let mut to_toggle: Option<String> = None;

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::initial(180.0).at_least(140.0))
            .column(Column::initial(70.0).at_least(50.0))
            .column(Column::initial(220.0).at_least(120.0))
            .column(Column::remainder().at_least(200.0))
            .column(Column::initial(70.0).at_least(50.0))
            .header(22.0, |mut header| {
                for label in ["Timestamp", "Level", "Function", "Message", "Status"] {
                    header.col(|ui| {
                        ui.label(egui::RichText::new(label).strong().small());
                    });
                }
            })
            .body(|mut body| {
                for entry in entries {
                    let key = row_key(entry);
                    let expanded = self.expanded.contains(&key);
                    let row_height = if expanded { 200.0 } else { 22.0 };

                    body.row(row_height, |mut row| {
                        row.col(|ui| {
                            ui.label(egui::RichText::new(format_ts(&entry.timestamp)).monospace().small());
                        });
                        row.col(|ui| {
                            let lvl = entry.level.as_deref().unwrap_or("");
                            ui.label(
                                egui::RichText::new(lvl).small().color(crate::theme::level_color(lvl)),
                            );
                        });
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(entry.function_name.as_deref().unwrap_or(""))
                                    .small()
                                    .monospace(),
                            );
                        });
                        row.col(|ui| {
                            // Clicking the message toggles expansion.
                            let label = if expanded {
                                egui::RichText::new(&entry.message).small()
                            } else {
                                egui::RichText::new(&entry.message)
                                    .small()
                                    .strong()
                            };
                            let resp = ui.add(
                                egui::Label::new(label).truncate().sense(egui::Sense::click()),
                            );
                            if resp.clicked() {
                                to_toggle = Some(key.clone());
                            }
                            if expanded {
                                ui.separator();
                                egui::ScrollArea::vertical()
                                    .max_height(160.0)
                                    .id_source(format!("expand-{key}"))
                                    .show(ui, |ui| {
                                        render_log_detail(ui, entry);
                                    });
                            }
                        });
                        row.col(|ui| {
                            if let Some(status) = entry.http_status {
                                let color = http_status_color(status);
                                ui.label(
                                    egui::RichText::new(status.to_string()).small().color(color),
                                );
                            }
                        });
                    });
                }
            });

        if let Some(key) = to_toggle {
            if !self.expanded.remove(&key) {
                self.expanded.insert(key);
            }
        }
    }

    fn loaded_ref(&self) -> Option<&LoadedState> {
        match &self.state {
            State::Loaded(l) => Some(l),
            _ => None,
        }
    }

    fn fire(&mut self, org_id: Uuid, api: &ApiClient, runtime: &tokio::runtime::Handle) {
        let (start, end) = self.preset.resolve_now();
        let query = LogQuery {
            search: non_empty(&self.search),
            level: None,
            log_group: None,
            function_name: None,
            http_path: None,
            http_status: None,
            trace_id: None,
            time_range: TimeRange { start, end },
            limit: Some(PAGE_SIZE),
            offset: Some(0),
            order_by: Some("desc".to_string()),
        };

        let (tx, rx): (Sender<FetchOutcome>, _) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.state = State::Loading;
        self.last_error = None;

        let api = api.clone();
        runtime.spawn(async move {
            let org = api.org(org_id);
            let outcome = match org.query_logs(&query).await {
                Ok(result) => {
                    let stats = org.log_stats(start, end).await.ok();
                    FetchOutcome::Ok { result, stats }
                }
                Err(e) => FetchOutcome::Err(render_error(e)),
            };
            let _ = tx.send(outcome);
        });
    }

    fn drain_pending(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.rx.as_ref() {
            if let Ok(outcome) = rx.try_recv() {
                self.rx = None;
                match outcome {
                    FetchOutcome::Ok { result, stats } => {
                        self.stats = stats;
                        self.state = State::Loaded(LoadedState {
                            entries: result.data,
                            total: result.total,
                            queried_at: Utc::now(),
                        });
                    }
                    FetchOutcome::Err(msg) => {
                        self.state = State::Failed;
                        self.last_error = Some(msg);
                    }
                }
                ctx.request_repaint();
            }
        }
    }
}

fn render_log_detail(ui: &mut egui::Ui, entry: &LogEntry) {
    egui::Grid::new(format!("detail-{}", entry.timestamp))
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            kv(ui, "log_group", entry.log_group.as_deref().unwrap_or(""));
            kv(ui, "log_stream", entry.log_stream.as_deref().unwrap_or(""));
            kv(ui, "request_id", entry.request_id.as_deref().unwrap_or(""));
            kv(ui, "trace_id", entry.trace_id.as_deref().unwrap_or(""));
            kv(
                ui,
                "duration_ms",
                entry
                    .duration_ms
                    .map(|d| format!("{d:.1}"))
                    .unwrap_or_default(),
            );
            if let Some(err) = &entry.error {
                if !err.is_empty() {
                    kv(ui, "error", err);
                }
            }
        });

    if let Some(parsed) = &entry.parsed_fields {
        if !parsed.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Parsed fields").small().strong());
            for (k, v) in parsed {
                kv(ui, k, v);
            }
        }
    }
}

fn kv(ui: &mut egui::Ui, k: &str, v: impl AsRef<str>) {
    ui.label(egui::RichText::new(k).monospace().small().color(egui::Color32::from_gray(150)));
    ui.label(egui::RichText::new(v.as_ref()).monospace().small());
    ui.end_row();
}

fn row_key(entry: &LogEntry) -> String {
    let stream = entry.log_stream.as_deref().unwrap_or("");
    format!("{}|{stream}", entry.timestamp)
}

fn format_ts(iso: &str) -> String {
    iso.replace('T', " ").trim_end_matches('Z').to_string()
}

fn http_status_color(status: i64) -> egui::Color32 {
    match status {
        500.. => crate::theme::smoo::RED,
        400..=499 => crate::theme::smoo::ORANGE,
        300..=399 => crate::theme::smoo::BLUE_400,
        _ => crate::theme::smoo::GRAY_500,
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i != 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn render_error(e: ApiError) -> String {
    match e {
        ApiError::Auth(a) => format!("auth: {a}"),
        ApiError::Http(h) => format!("network: {h}"),
        ApiError::Url(u) => format!("url: {u}"),
        ApiError::Status { status, body } => {
            let body_short = body.chars().take(200).collect::<String>();
            format!("{status}: {body_short}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_formats_grouping() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(42), "42");
        assert_eq!(thousands(1_234), "1,234");
        assert_eq!(thousands(12_345_678), "12,345,678");
    }

    #[test]
    fn non_empty_trims_and_drops_blank() {
        assert_eq!(non_empty("  "), None);
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty("  hi "), Some("hi".to_string()));
    }

    #[test]
    fn http_status_color_buckets() {
        assert_eq!(http_status_color(200), crate::theme::smoo::GRAY_500);
        assert_eq!(http_status_color(301), crate::theme::smoo::BLUE_400);
        assert_eq!(http_status_color(404), crate::theme::smoo::ORANGE);
        assert_eq!(http_status_color(503), crate::theme::smoo::RED);
    }
}
