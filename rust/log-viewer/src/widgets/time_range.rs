//! Time-range picker shared by Logs + Metrics views.
//!
//! Matches the preset set used by `apps/web/components/observability/time-range-picker.tsx`
//! so users see the same options here as in the browser.

use chrono::{DateTime, Duration, Utc};
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimePreset {
    Last15m,
    Last1h,
    Last6h,
    Last24h,
    Last7d,
}

impl TimePreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Last15m => "15m",
            Self::Last1h => "1h",
            Self::Last6h => "6h",
            Self::Last24h => "24h",
            Self::Last7d => "7d",
        }
    }

    pub fn duration(self) -> Duration {
        match self {
            Self::Last15m => Duration::minutes(15),
            Self::Last1h => Duration::hours(1),
            Self::Last6h => Duration::hours(6),
            Self::Last24h => Duration::hours(24),
            Self::Last7d => Duration::days(7),
        }
    }

    pub const ALL: &'static [TimePreset] = &[
        Self::Last15m,
        Self::Last1h,
        Self::Last6h,
        Self::Last24h,
        Self::Last7d,
    ];

    pub fn resolve_now(self) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc::now();
        (now - self.duration(), now)
    }
}

impl Default for TimePreset {
    fn default() -> Self {
        Self::Last1h
    }
}

/// Inline preset picker. Returns `true` when the user changed the selection.
pub fn preset_picker(ui: &mut egui::Ui, selected: &mut TimePreset) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for preset in TimePreset::ALL {
            if ui
                .selectable_label(*selected == *preset, preset.label())
                .clicked()
            {
                *selected = *preset;
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_resolve_to_now_minus_duration() {
        let (start, end) = TimePreset::Last1h.resolve_now();
        let delta = end - start;
        assert_eq!(delta.num_minutes(), 60);
    }

    #[test]
    fn labels_match_web_presets() {
        // Don't change these without coordinating with apps/web's
        // time-range-picker.tsx — users should see the same options.
        let labels: Vec<_> = TimePreset::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(labels, vec!["15m", "1h", "6h", "24h", "7d"]);
    }
}
