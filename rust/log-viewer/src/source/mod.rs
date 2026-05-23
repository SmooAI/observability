//! `DataSource` abstraction over local `.smooai-logs/` files and remote SmooAI
//! observability API. See `docs/Engineering/Rust-Desktop-Observability-Viewer.md`
//! (phase 1 keeps the monolithic local pipeline in `main.rs`; this module will
//! be filled in during phase 2 alongside the API client).

#![allow(dead_code)]

use std::path::PathBuf;
use uuid::Uuid;

/// Where a tab's data comes from.
#[derive(Clone, Debug)]
pub enum SourceKind {
    /// Watch and ingest `.smooai-logs/` files under `root`.
    LocalFs { root: PathBuf },
    /// Hit `https://api.smoo.ai/organizations/{org_id}/observability/*`.
    Remote {
        org_id: Uuid,
        base_url: url::Url,
        /// `client_id` lives here; the secret is in the OS keychain.
        client_id: String,
        label: String,
    },
}
