//! Edge pipeline for the api-prime data plane.
//!
//! ADR-017 (API Prime / programmable edge) splits per-request work into
//! discrete pipeline stages that run BEFORE we dispatch to a per-route
//! backend mode (proxy / cache / implement):
//!
//! ```text
//! request → route_table.lookup → auth.verify → ratelimit.check →
//!     schema.validate_request → dispatcher.dispatch
//! ```
//!
//! Submodules:
//! - [`types`]     — `RouteEntry` mirror of the controller's TS schema.
//! - [`route_table`] — `apr:route:*` reader + RCU-swap on `apr:config-bump`.
//! - [`auth`]      — JWT + M2M verification, produces `EdgeAuthContext`.
//! - [`ratelimit`] — Valkey sliding-window enforcement.
//! - [`schema`]    — request/response schema validation (stub in v1).
//! - [`cache`]     — L1 in-proc LRU + L2 Valkey with SWR semantics.
//! - [`pubsub`]    — subscriber for `apr:config-bump` + `apr:invalidate`.
//! - [`proxy`]     — direct Lambda invoke (no API Gateway hop).
//! - [`edge_attest`] — HMAC-signed attestation payload for the trust boundary.
//! - [`implement`] — static dispatch into our in-process Rust handlers.
//! - [`dispatcher`] — the per-request pipeline that ties it all together.
//! - [`debug`]     — dev-only response headers (`X-Smoo-Cache-Status` etc.).
//! - [`ctx`]       — the shared `EdgeContext` injected into the axum service.

pub mod auth;
pub mod cache;
pub mod ctx;
pub mod debug;
pub mod dispatcher;
pub mod edge_attest;
pub mod implement;
pub mod proxy;
pub mod pubsub;
pub mod ratelimit;
pub mod route_table;
pub mod schema;
pub mod types;
