//! Installs SmooAI's branded fonts as the default `Proportional` + `Monospace`
//! families in egui. Called once from the `eframe::CreationContext` setup
//! before any UI runs.
//!
//! Bundled:
//! - **Inter Variable** (~860 KB) — OFL, https://rsms.me/inter/
//! - **JetBrains Mono Regular** (~265 KB) — OFL, https://www.jetbrains.com/lp/mono/
//!
//! Both files live under `assets/fonts/` and are `include_bytes!`'d at compile
//! time so the binary remains a single-file install.

use eframe::egui::{FontData, FontDefinitions, FontFamily};

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Variable.ttf");
const MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

pub fn install(ctx: &eframe::egui::Context) {
    let mut fonts = FontDefinitions::default();

    fonts
        .font_data
        .insert("inter".to_owned(), FontData::from_static(INTER));
    fonts
        .font_data
        .insert("jbmono".to_owned(), FontData::from_static(MONO));

    // Front of the queue → primary face. Keep the egui default fallback chain
    // behind us so emoji + missing-glyph fall-through still works.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jbmono".to_owned());

    ctx.set_fonts(fonts);
}
