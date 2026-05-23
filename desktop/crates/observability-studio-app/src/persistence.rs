//! Org registry — non-secret metadata about each connected org. Lives at
//! `<config_dir>/org-registry.json`; secrets live in the OS keychain via
//! `observability_studio_client::auth`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const REGISTRY_FILE: &str = "org-registry.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgEntry {
    pub org_id: Uuid,
    pub label: String,
    pub base_url: String,
    /// Non-secret — duplicated here so the Settings list can show a preview
    /// without an extra keychain read on every render.
    pub client_id_preview: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OrgRegistry {
    #[serde(default)]
    pub entries: Vec<OrgEntry>,
    /// Skip the path field on serde so the file stays clean; populated by
    /// `load_or_default`.
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl OrgRegistry {
    pub fn load_or_default() -> Self {
        let path = match config_path() {
            Some(p) => p,
            None => return Self::default(),
        };
        if !path.exists() {
            return Self { entries: vec![], path: Some(path) };
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return Self { entries: vec![], path: Some(path) },
        };
        let mut me: Self = serde_json::from_str(&raw).unwrap_or_default();
        me.path = Some(path);
        me
    }

    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(raw) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, raw);
        }
    }

    pub fn upsert(&mut self, entry: OrgEntry) {
        if let Some(existing) =
            self.entries.iter_mut().find(|e| e.org_id == entry.org_id)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    pub fn remove(&mut self, org_id: Uuid) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.org_id != org_id);
        self.entries.len() != before
    }
}

fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("ai", "Smoo", "ObservabilityStudio")
        .map(|d| d.config_dir().join(REGISTRY_FILE))
}
