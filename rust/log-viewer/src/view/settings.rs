//! Settings view — manage M2M orgs (add, verify, remove).
//!
//! Renders inside an `egui::Window` toggled from the top toolbar. Owns
//! transient form state and the org registry; the actual creds live in the OS
//! keychain via [`crate::auth::Keychain`].

use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{AuthError, AuthManager, Credentials, Keychain};

/// One row in the user's list of remote orgs. Secrets live in the keychain;
/// only the public identifiers and a user-chosen label are persisted to disk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrgEntry {
    pub org_id: Uuid,
    pub label: String,
    pub base_url: String,
    /// `client_id` is duplicated here (non-secret) so we can display it in the
    /// settings panel without an extra keychain read on each frame.
    pub client_id_preview: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OrgRegistry {
    pub entries: Vec<OrgEntry>,
}

impl OrgRegistry {
    pub fn remove(&mut self, org_id: Uuid) {
        self.entries.retain(|e| e.org_id != org_id);
    }

    pub fn contains(&self, org_id: Uuid) -> bool {
        self.entries.iter().any(|e| e.org_id == org_id)
    }
}

/// Transient form state for the "Add Org" wizard. Cleared after a successful
/// verify-and-add.
#[derive(Default)]
pub struct AddOrgForm {
    pub org_id_text: String,
    pub label: String,
    pub base_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub busy: bool,
    pub last_error: Option<String>,
    pub verify_rx: Option<Receiver<VerifyOutcome>>,
}

pub enum VerifyOutcome {
    Ok { org_id: Uuid, entry: OrgEntry, credentials: Credentials },
    Err(String),
}

/// Top-level Settings panel state. Composed into the App.
pub struct SettingsState {
    pub open: bool,
    pub registry: OrgRegistry,
    pub add: AddOrgForm,
    /// Per-org status badge text (cleared on "Remove").
    pub statuses: std::collections::HashMap<Uuid, String>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            registry: OrgRegistry::default(),
            add: AddOrgForm::default(),
            statuses: std::collections::HashMap::new(),
        }
    }
}

impl SettingsState {
    /// Render the Settings window. Returns `true` if the registry changed and
    /// the caller should re-persist state.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        auth: &AuthManager,
        runtime: &tokio::runtime::Handle,
    ) -> bool {
        if !self.open {
            return false;
        }
        let mut changed = false;
        let mut open = self.open;
        let mut to_remove: Option<Uuid> = None;

        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.heading("Remote orgs");
                ui.label(
                    "Connect to api.smoo.ai with an M2M client_id / client_secret \
                     pair (mint one from the SmooAI dashboard or via mint-customer-m2m-key.ts). \
                     Secrets are stored in the OS keychain — never on disk.",
                );
                ui.add_space(8.0);

                if self.registry.entries.is_empty() {
                    ui.label(
                        egui::RichText::new("No orgs added yet.")
                            .italics()
                            .color(egui::Color32::from_gray(160)),
                    );
                } else {
                    egui::Grid::new("orgs_grid")
                        .num_columns(4)
                        .spacing([16.0, 6.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Label");
                            ui.strong("Org ID");
                            ui.strong("client_id");
                            ui.strong("");
                            ui.end_row();
                            for entry in &self.registry.entries {
                                ui.label(&entry.label);
                                ui.label(
                                    egui::RichText::new(entry.org_id.to_string())
                                        .monospace()
                                        .small(),
                                );
                                ui.label(
                                    egui::RichText::new(&entry.client_id_preview)
                                        .monospace()
                                        .small(),
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("Remove").clicked() {
                                        to_remove = Some(entry.org_id);
                                    }
                                    if let Some(status) = self.statuses.get(&entry.org_id) {
                                        ui.label(
                                            egui::RichText::new(status).small().color(
                                                egui::Color32::from_rgb(120, 180, 120),
                                            ),
                                        );
                                    }
                                });
                                ui.end_row();
                            }
                        });
                }

                ui.add_space(12.0);
                ui.separator();
                ui.heading("Add org");
                add_org_form(ui, &mut self.add, auth, runtime);
            });

        // Process verify outcomes
        if let Some(rx) = self.add.verify_rx.as_ref() {
            if let Ok(outcome) = rx.try_recv() {
                self.add.busy = false;
                self.add.verify_rx = None;
                match outcome {
                    VerifyOutcome::Ok { org_id, entry, credentials } => {
                        // Commit creds to keychain, then push to registry.
                        let kc = Keychain::new();
                        match kc.store(org_id, &credentials) {
                            Ok(()) => {
                                self.registry.entries.push(entry);
                                self.statuses.insert(org_id, "✓ Verified".into());
                                self.add = AddOrgForm::default();
                                changed = true;
                            }
                            Err(e) => {
                                self.add.last_error =
                                    Some(format!("Keychain write failed: {e}"));
                            }
                        }
                    }
                    VerifyOutcome::Err(msg) => {
                        self.add.last_error = Some(msg);
                    }
                }
                ctx.request_repaint();
            }
        }

        if let Some(org_id) = to_remove {
            // Best-effort keychain delete.
            let _ = Keychain::new().remove(org_id);
            auth.clear_override(org_id);
            auth.invalidate(org_id);
            self.registry.remove(org_id);
            self.statuses.remove(&org_id);
            changed = true;
        }

        self.open = open;
        changed
    }
}

fn add_org_form(
    ui: &mut egui::Ui,
    form: &mut AddOrgForm,
    auth: &AuthManager,
    runtime: &tokio::runtime::Handle,
) {
    egui::Grid::new("add_org_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Label");
            ui.text_edit_singleline(&mut form.label);
            ui.end_row();

            ui.label("Org ID (UUID)");
            ui.text_edit_singleline(&mut form.org_id_text);
            ui.end_row();

            ui.label("Base URL");
            let resp = ui.text_edit_singleline(&mut form.base_url);
            if form.base_url.is_empty() && !resp.has_focus() {
                form.base_url = crate::auth::API_BASE.to_string();
            }
            ui.end_row();

            ui.label("client_id");
            ui.text_edit_singleline(&mut form.client_id);
            ui.end_row();

            ui.label("client_secret");
            ui.add(egui::TextEdit::singleline(&mut form.client_secret).password(true));
            ui.end_row();
        });

    ui.add_space(8.0);
    if let Some(err) = &form.last_error {
        ui.colored_label(egui::Color32::from_rgb(220, 100, 100), err);
    }
    ui.horizontal(|ui| {
        let can_submit = !form.busy
            && !form.label.trim().is_empty()
            && !form.client_id.trim().is_empty()
            && !form.client_secret.trim().is_empty()
            && !form.org_id_text.trim().is_empty();

        if ui
            .add_enabled(can_submit, egui::Button::new("Verify & save"))
            .clicked()
        {
            // Validate UUID up front to avoid spinning a network call on garbage input.
            match Uuid::parse_str(form.org_id_text.trim()) {
                Ok(org_id) => {
                    let entry = OrgEntry {
                        org_id,
                        label: form.label.trim().to_string(),
                        base_url: if form.base_url.trim().is_empty() {
                            crate::auth::API_BASE.to_string()
                        } else {
                            form.base_url.trim().to_string()
                        },
                        client_id_preview: form.client_id.trim().to_string(),
                    };
                    let creds = Credentials {
                        client_id: form.client_id.trim().to_string(),
                        client_secret: form.client_secret.trim().to_string(),
                    };

                    let (tx, rx): (Sender<VerifyOutcome>, _) = std::sync::mpsc::channel();
                    form.verify_rx = Some(rx);
                    form.busy = true;
                    form.last_error = None;

                    let auth = auth.clone();
                    let ctx = ui.ctx().clone();
                    runtime.spawn(async move {
                        let outcome = match auth
                            .verify(&creds.client_id, &creds.client_secret)
                            .await
                        {
                            Ok(_) => VerifyOutcome::Ok { org_id, entry, credentials: creds },
                            Err(AuthError::TokenEndpoint(status)) => {
                                VerifyOutcome::Err(format!(
                                    "Token endpoint rejected the credentials ({status})"
                                ))
                            }
                            Err(other) => VerifyOutcome::Err(format!("{other}")),
                        };
                        let _ = tx.send(outcome);
                        ctx.request_repaint();
                    });
                }
                Err(e) => {
                    form.last_error = Some(format!("Invalid org UUID: {e}"));
                }
            }
        }

        if form.busy {
            ui.spinner();
            ui.label("Verifying…");
        }
    });
}
