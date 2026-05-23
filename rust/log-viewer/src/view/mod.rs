//! Per-view modules — one file per dashboard tab, all rendering through egui.
//! Phase 1 kept the local-source rendering in `main.rs`; phase 2 adds the
//! Settings panel and the auth/api wiring it depends on.

#![allow(dead_code)]

pub mod settings;
