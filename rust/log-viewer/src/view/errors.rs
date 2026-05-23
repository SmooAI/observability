//! Remote Errors view — mirrors `apps/web/components/observability/errors/{error-list,error-detail}.tsx`.
//!
//! Two modes: a list of error groups (cards), and a detail pane for one group.
//! Status mutations (mark resolved / mark muted) hit `PATCH /errors/{id}` and
//! optimistically update the local copy.

use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;
use uuid::Uuid;

use crate::api::errors::{
    ErrorDetail, ErrorGroup, ErrorListParams, ErrorPage, ErrorPatch, ErrorStatus,
};
use crate::api::ApiClient;
use crate::widgets::{
    kpi_card::{cards_row, Kpi},
    stack_frame,
};

#[derive(Default)]
pub struct RemoteErrorsView {
    pub org_id: Option<Uuid>,
    pub status_filter: Option<ErrorStatus>,
    pub environment_filter: String,
    mode: Mode,
    list_state: ListState,
    detail_state: DetailState,
    list_rx: Option<Receiver<ListOutcome>>,
    detail_rx: Option<Receiver<DetailOutcome>>,
    patch_rx: Option<Receiver<PatchOutcome>>,
    last_error: Option<String>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Mode {
    #[default]
    List,
    Detail,
}

#[derive(Default)]
struct ListState {
    loading: bool,
    groups: Vec<ErrorGroup>,
    next_cursor: Option<String>,
}

#[derive(Default)]
struct DetailState {
    loading: bool,
    detail: Option<ErrorDetail>,
}

enum ListOutcome {
    Ok(ErrorPage),
    Err(String),
}

enum DetailOutcome {
    Ok(ErrorDetail),
    Err(String),
}

enum PatchOutcome {
    Ok(ErrorGroup),
    Err(String),
}

impl RemoteErrorsView {
    pub fn for_org(org_id: Uuid) -> Self {
        Self {
            org_id: Some(org_id),
            status_filter: Some(ErrorStatus::Unresolved),
            environment_filter: String::new(),
            ..Default::default()
        }
    }

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

        self.drain(ui.ctx());

        match self.mode {
            Mode::List => self.render_list(ui, org_id, api, runtime),
            Mode::Detail => self.render_detail(ui, org_id, api, runtime),
        }
    }

    fn render_list(
        &mut self,
        ui: &mut egui::Ui,
        org_id: Uuid,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        // Filter strip.
        ui.horizontal(|ui| {
            ui.label("Status:");
            for (label, value) in [
                ("Unresolved", Some(ErrorStatus::Unresolved)),
                ("Resolved", Some(ErrorStatus::Resolved)),
                ("Muted", Some(ErrorStatus::Muted)),
                ("All", None),
            ] {
                if ui
                    .selectable_label(self.status_filter == value, label)
                    .clicked()
                {
                    self.status_filter = value;
                    self.fire_list(org_id, api, runtime);
                }
            }
            ui.separator();
            ui.label("Env:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.environment_filter)
                    .hint_text("any")
                    .desired_width(140.0),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.fire_list(org_id, api, runtime);
            }
            if ui.button("⟳ Refresh").clicked() {
                self.fire_list(org_id, api, runtime);
            }
            if self.list_state.loading {
                ui.spinner();
            }
        });

        // Auto-fire on first frame.
        if self.list_state.groups.is_empty() && !self.list_state.loading && self.list_rx.is_none()
        {
            self.fire_list(org_id, api, runtime);
        }

        ui.add_space(4.0);
        if let Some(err) = &self.last_error {
            ui.colored_label(egui::Color32::from_rgb(220, 100, 100), err);
            ui.add_space(4.0);
        }

        // Card list (avoids borrowing self while iterating — clone the small Vec).
        let groups = self.list_state.groups.clone();
        let mut click_id: Option<Uuid> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for group in &groups {
                if render_group_card(ui, group).clicked() {
                    if let Ok(id) = Uuid::parse_str(&group.id) {
                        click_id = Some(id);
                    }
                }
                ui.add_space(4.0);
            }
        });
        if let Some(id) = click_id {
            self.open_detail(id, org_id, api, runtime);
        }
    }

    fn render_detail(
        &mut self,
        ui: &mut egui::Ui,
        org_id: Uuid,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.mode = Mode::List;
                self.detail_state.detail = None;
            }
            if self.detail_state.loading {
                ui.spinner();
            }
        });
        ui.add_space(8.0);

        // Clone detail snapshot to avoid borrowing self while we render +
        // potentially fire mutations.
        let detail = match self.detail_state.detail.clone() {
            Some(d) => d,
            None => {
                ui.centered_and_justified(|ui| ui.label("Loading detail…"));
                return;
            }
        };

        let group = &detail.group;
        ui.horizontal_wrapped(|ui| {
            ui.label(level_badge(&group.level));
            ui.label(env_badge(&group.environment));
            ui.label(status_badge(group.status));
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(&group.title).heading());
        if let Some(culprit) = &group.culprit {
            ui.label(
                egui::RichText::new(culprit)
                    .monospace()
                    .small()
                    .color(egui::Color32::from_gray(170)),
            );
        }
        ui.add_space(8.0);

        cards_row(
            ui,
            &[
                Kpi {
                    label: "Events",
                    value: group.event_count.to_string(),
                    color: None,
                },
                Kpi {
                    label: "Users",
                    value: group.user_count.to_string(),
                    color: None,
                },
                Kpi {
                    label: "Last seen",
                    value: short_ts(&group.last_seen_at),
                    color: None,
                },
            ],
        );
        ui.add_space(8.0);

        let mut new_status: Option<ErrorStatus> = None;
        ui.horizontal(|ui| {
            if group.status != ErrorStatus::Resolved
                && ui.button("✓ Mark resolved").clicked()
            {
                new_status = Some(ErrorStatus::Resolved);
            }
            if group.status != ErrorStatus::Muted && ui.button("🔇 Mute").clicked() {
                new_status = Some(ErrorStatus::Muted);
            }
            if group.status != ErrorStatus::Unresolved
                && ui.button("↩ Reopen").clicked()
            {
                new_status = Some(ErrorStatus::Unresolved);
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.heading("Recent events");
        egui::ScrollArea::vertical().show(ui, |ui| {
            for event in &detail.recent_events {
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&event.event_id).monospace().small(),
                            );
                            ui.label(
                                egui::RichText::new(short_ts(&event.occurred_at))
                                    .small()
                                    .color(egui::Color32::from_gray(160)),
                            );
                        });
                        if let Some(msg) = &event.message {
                            ui.label(egui::RichText::new(msg).small());
                        }
                        // Pull frames if the exception payload carries them in
                        // the standard SDK shape.
                        if let Some(frames) = extract_first_stacktrace(&event.exception) {
                            ui.add_space(4.0);
                            stack_frame::render_frames(ui, &frames);
                        }
                    });
                ui.add_space(4.0);
            }
        });

        if let Some(status) = new_status {
            self.fire_patch(org_id, &group.id, status, api, runtime);
        }
    }

    fn fire_list(&mut self, org_id: Uuid, api: &ApiClient, runtime: &tokio::runtime::Handle) {
        let env = if self.environment_filter.trim().is_empty() {
            None
        } else {
            Some(self.environment_filter.trim().to_string())
        };
        let params = ErrorListParams {
            environment: env,
            status: self.status_filter,
            cursor: None,
            limit: Some(50),
        };
        let (tx, rx): (Sender<ListOutcome>, _) = std::sync::mpsc::channel();
        self.list_rx = Some(rx);
        self.list_state.loading = true;
        self.last_error = None;
        let api = api.clone();
        runtime.spawn(async move {
            let outcome = match api.org(org_id).list_error_groups(&params).await {
                Ok(page) => ListOutcome::Ok(page),
                Err(e) => ListOutcome::Err(format!("{e:?}")),
            };
            let _ = tx.send(outcome);
        });
    }

    fn open_detail(
        &mut self,
        group_id: Uuid,
        org_id: Uuid,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        self.mode = Mode::Detail;
        self.detail_state.loading = true;
        self.detail_state.detail = None;
        let (tx, rx): (Sender<DetailOutcome>, _) = std::sync::mpsc::channel();
        self.detail_rx = Some(rx);
        let api = api.clone();
        runtime.spawn(async move {
            let outcome = match api.org(org_id).get_error_group(group_id).await {
                Ok(d) => DetailOutcome::Ok(d),
                Err(e) => DetailOutcome::Err(format!("{e:?}")),
            };
            let _ = tx.send(outcome);
        });
    }

    fn fire_patch(
        &mut self,
        org_id: Uuid,
        group_id_str: &str,
        status: ErrorStatus,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        let Ok(group_id) = Uuid::parse_str(group_id_str) else {
            self.last_error = Some("invalid group id".into());
            return;
        };
        let patch = ErrorPatch {
            status: Some(status),
            assigned_user_id: None,
        };
        let (tx, rx): (Sender<PatchOutcome>, _) = std::sync::mpsc::channel();
        self.patch_rx = Some(rx);
        let api = api.clone();
        runtime.spawn(async move {
            let outcome = match api.org(org_id).update_error_group(group_id, &patch).await {
                Ok(g) => PatchOutcome::Ok(g),
                Err(e) => PatchOutcome::Err(format!("{e:?}")),
            };
            let _ = tx.send(outcome);
        });
    }

    fn drain(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.list_rx.as_ref() {
            if let Ok(outcome) = rx.try_recv() {
                self.list_rx = None;
                self.list_state.loading = false;
                match outcome {
                    ListOutcome::Ok(page) => {
                        self.list_state.groups = page.groups;
                        self.list_state.next_cursor = page.next_cursor;
                    }
                    ListOutcome::Err(msg) => self.last_error = Some(msg),
                }
                ctx.request_repaint();
            }
        }
        if let Some(rx) = self.detail_rx.as_ref() {
            if let Ok(outcome) = rx.try_recv() {
                self.detail_rx = None;
                self.detail_state.loading = false;
                match outcome {
                    DetailOutcome::Ok(d) => self.detail_state.detail = Some(d),
                    DetailOutcome::Err(msg) => {
                        self.last_error = Some(msg);
                        self.mode = Mode::List;
                    }
                }
                ctx.request_repaint();
            }
        }
        if let Some(rx) = self.patch_rx.as_ref() {
            if let Ok(outcome) = rx.try_recv() {
                self.patch_rx = None;
                match outcome {
                    PatchOutcome::Ok(updated) => {
                        if let Some(detail) = self.detail_state.detail.as_mut() {
                            if detail.group.id == updated.id {
                                detail.group = updated.clone();
                            }
                        }
                        // Patch the list copy too.
                        if let Some(row) =
                            self.list_state.groups.iter_mut().find(|g| g.id == updated.id)
                        {
                            *row = updated;
                        }
                    }
                    PatchOutcome::Err(msg) => self.last_error = Some(msg),
                }
                ctx.request_repaint();
            }
        }
    }
}

fn render_group_card(ui: &mut egui::Ui, group: &ErrorGroup) -> egui::Response {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(level_badge(&group.level));
                ui.label(env_badge(&group.environment));
                ui.label(status_badge(group.status));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(short_ts(&group.last_seen_at))
                            .small()
                            .color(egui::Color32::from_gray(150)),
                    );
                });
            });
            ui.label(egui::RichText::new(&group.title).strong());
            if let Some(culprit) = &group.culprit {
                ui.label(
                    egui::RichText::new(culprit)
                        .monospace()
                        .small()
                        .color(egui::Color32::from_gray(160)),
                );
            }
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} events", group.event_count))
                        .small(),
                );
                ui.label(
                    egui::RichText::new(format!("{} users", group.user_count))
                        .small()
                        .color(egui::Color32::from_gray(150)),
                );
            });
        })
        .response
        .interact(egui::Sense::click())
}

fn level_badge(level: &str) -> egui::RichText {
    let color = match level.to_ascii_lowercase().as_str() {
        "fatal" | "error" => crate::theme::smoo::RED,
        "warn" | "warning" => crate::theme::smoo::ORANGE,
        _ => crate::theme::smoo::BLUE_400,
    };
    egui::RichText::new(level.to_uppercase()).small().color(color).strong()
}

fn env_badge(env: &str) -> egui::RichText {
    egui::RichText::new(env).small().color(egui::Color32::from_gray(160))
}

fn status_badge(status: ErrorStatus) -> egui::RichText {
    let (text, color) = match status {
        ErrorStatus::Unresolved => ("unresolved", crate::theme::smoo::RED),
        ErrorStatus::Resolved => ("resolved", crate::theme::smoo::GREEN),
        ErrorStatus::Muted => ("muted", crate::theme::smoo::GRAY_500),
    };
    egui::RichText::new(text).small().color(color)
}

fn short_ts(iso: &str) -> String {
    iso.replace('T', " ").trim_end_matches('Z').to_string()
}

/// Pulls the first frames array out of an `exception` JSON value, matching the
/// SDK shape `exception[0].stacktrace.frames[]`.
fn extract_first_stacktrace(exception: &Option<serde_json::Value>) -> Option<Vec<stack_frame::Frame>> {
    let exc = exception.as_ref()?;
    let arr = exc.as_array()?;
    let first = arr.first()?;
    let frames = first.get("stacktrace")?.get("frames")?;
    serde_json::from_value::<Vec<stack_frame::Frame>>(frames.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_first_stacktrace_handles_sdk_shape() {
        let exc = json!([
            {
                "stacktrace": {
                    "frames": [
                        {"function": "doIt", "filename": "foo.ts", "lineno": 5},
                        {"function": "main", "filename": "main.ts", "lineno": 1}
                    ]
                }
            }
        ]);
        let frames = extract_first_stacktrace(&Some(exc)).expect("frames");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].function.as_deref(), Some("doIt"));
    }

    #[test]
    fn extract_first_stacktrace_none_when_missing() {
        assert!(extract_first_stacktrace(&None).is_none());
        assert!(extract_first_stacktrace(&Some(json!({}))).is_none());
        assert!(extract_first_stacktrace(&Some(json!([]))).is_none());
        assert!(extract_first_stacktrace(&Some(json!([{"other": 1}]))).is_none());
    }

    #[test]
    fn status_badge_text() {
        // RichText doesn't impl Debug; .text() returns the underlying string.
        assert_eq!(status_badge(ErrorStatus::Unresolved).text(), "unresolved");
        assert_eq!(status_badge(ErrorStatus::Resolved).text(), "resolved");
        assert_eq!(status_badge(ErrorStatus::Muted).text(), "muted");
    }
}
