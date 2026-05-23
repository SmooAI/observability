//! Typed client for `https://api.smoo.ai/organizations/{org_id}/observability/*`.
//!
//! Filled in during phase 2 of SMOODEV-1175. Response types mirror
//! `apps/web/components/services/observability-service.ts` for 1:1 parity with
//! the canonical browser dashboard. See
//! `docs/Engineering/Rust-Desktop-Observability-Viewer.md` §5.2.

#![allow(dead_code)]

pub mod logs;
pub mod errors;
pub mod metrics;
pub mod connections;
