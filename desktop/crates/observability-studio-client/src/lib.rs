//! Typed client for the SmooAI Observability API.
//!
//! - `auth` — M2M `client_credentials` exchange + OS-keychain credential
//!   storage + in-memory bearer cache.
//! - `api` — typed wrappers around `https://api.smoo.ai/organizations/{org}/observability/*`.
//!
//! The shape mirrors the (now-deprecated) egui binary's `client` modules; the
//! Dioxus app calls these directly from view components rather than going
//! through a Tauri-style IPC layer.

#![warn(clippy::all)]

pub mod api;
pub mod auth;

pub use auth::{AuthError, AuthManager, TokenResponse, API_BASE, TOKEN_URL};
