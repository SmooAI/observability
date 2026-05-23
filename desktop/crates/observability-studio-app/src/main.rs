//! Window bootstrap. Mirrors smooblue's main.rs pattern — keep this thin so
//! all testable logic lives in `lib.rs`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxus::prelude::*;
use dioxus_desktop::tao::dpi::LogicalSize;
use dioxus_desktop::{Config, WindowBuilder};

fn main() {
    let cfg = Config::new().with_window(
        WindowBuilder::new()
            .with_title("SmooAI Observability Studio")
            .with_inner_size(LogicalSize::new(1280.0, 820.0))
            .with_min_inner_size(LogicalSize::new(960.0, 640.0)),
    );

    LaunchBuilder::desktop()
        .with_cfg(cfg)
        .launch(observability_studio_app::App);
}
