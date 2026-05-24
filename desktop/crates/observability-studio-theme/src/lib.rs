//! Observability Studio's CSS layer.
//!
//! Composes two sources of truth:
//!
//! 1. [`smooai_ui::STYLES`] — the cross-language brand foundation
//!    (tokens, `.btn`, `.card`, `.modal__sheet`, `.brand-badge`, monogram,
//!    reset). Shared with smooblue + future SmooAI Rust desktop apps.
//! 2. [`APP_STYLES`] — observability-studio's own shell layout + view-
//!    specific components (`.shell`, `.studio-rail`, `.view-header`,
//!    `.welcome__*`, `.org-row`, `.field`, `.status-bar`).
//!
//! Apps embed BOTH via [`STYLES`] (concatenated brand+app) at the root
//! component. Brand styles win the cascade on conflict — the app intentionally
//! never overrides them.

/// The smooai-ui brand foundation (shared across SmooAI Rust desktop apps).
pub const BRAND_STYLES: &str = smooai_ui::STYLES;

/// Observability Studio's own shell + view-specific CSS layer.
pub const APP_STYLES: &str = include_str!("../../../assets/styles.css");

/// Convenience accessor — re-export the brand monogram so views in this app
/// don't need to depend on `smooai-ui` directly.
pub const MONOGRAM_SVG: &str = smooai_ui::MONOGRAM_SVG;

/// Re-export brand token constants for code paths that need a color outside
/// of CSS (custom-painted widgets, native menu chrome, chart libraries).
pub use smooai_ui::tokens;

