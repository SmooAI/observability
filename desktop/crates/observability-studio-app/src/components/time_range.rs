//! Time-range preset picker. Matches the set used by
//! `apps/web/components/observability/time-range-picker.tsx` so users see the
//! same options in the desktop as in the browser dashboard.

use chrono::{DateTime, Duration, Utc};
use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Props, Clone, PartialEq)]
pub struct TimeRangePickerProps {
    pub selected: TimePreset,
    pub on_change: EventHandler<TimePreset>,
}

#[component]
pub fn TimeRangePicker(props: TimeRangePickerProps) -> Element {
    rsx! {
        div { class: "time-range",
            for preset in TimePreset::ALL.iter().copied() {
                {
                    let active = preset == props.selected;
                    let class = if active { "time-range__btn time-range__btn--active" } else { "time-range__btn" };
                    let label = preset.label();
                    let on_change = props.on_change;
                    rsx! {
                        button {
                            key: "{label}",
                            class: "{class}",
                            onclick: move |_| on_change.call(preset),
                            "{label}"
                        }
                    }
                }
            }
        }
    }
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
    fn labels_match_web_picker() {
        // Don't change these without coordinating with apps/web's
        // time-range-picker.tsx — users should see the same options in both
        // surfaces.
        let labels: Vec<_> = TimePreset::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(labels, vec!["15m", "1h", "6h", "24h", "7d"]);
    }
}
