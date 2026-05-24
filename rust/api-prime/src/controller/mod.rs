//! Control-plane modules for the api-prime-controller binary.
//!
//! See `src/bin/api-prime-controller.rs` and ADR-017 for the architecture
//! context. This module tree is intentionally only used by the controller
//! binary — the data-plane binary (`api-prime.rs`) does not import it.

pub mod admin;
pub mod internal;
pub mod manifest_loader;
pub mod pubsub;
pub mod reconcile;
pub mod sst_outputs;
pub mod types;

/// Set of Rust handler names registered in this build. PUT
/// `/admin/v1/routes/:id/mode` to `implement` rejects routes whose
/// `implement.rustHandler` is not in this set.
///
/// Phase 1 placeholder list — Castor will expand this as handlers ship in
/// `src/handlers/`. See SMOODEV-1283.
pub const REGISTERED_RUST_HANDLERS: &[&str] = &[
    "profile",
    "organizations",
    "organization_features",
    "organization_products",
];

/// `true` if the handler name is registered in this build.
pub fn is_registered_handler(name: &str) -> bool {
    REGISTERED_RUST_HANDLERS.contains(&name)
}
