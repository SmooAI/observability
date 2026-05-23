//! KPI tiles — the four metric pill rows that float above any data table
//! (Total Logs / Errors / Error Rate / P95 Duration today; reused by Errors +
//! Metrics views later).

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct KpiRowProps {
    pub tiles: Vec<KpiTile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KpiTile {
    pub label: String,
    pub value: String,
    /// One of: `""` (default foreground), `"accent"`, `"warning"`,
    /// `"destructive"`. The CSS owns the colour mapping so we don't ship
    /// inline-style colours from Rust.
    pub tone: KpiTone,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum KpiTone {
    #[default]
    Default,
    Accent,
    Warning,
    Destructive,
}

impl KpiTone {
    fn class(self) -> &'static str {
        match self {
            Self::Default => "kpi-tile__value",
            Self::Accent => "kpi-tile__value kpi-tile__value--accent",
            Self::Warning => "kpi-tile__value kpi-tile__value--warning",
            Self::Destructive => "kpi-tile__value kpi-tile__value--destructive",
        }
    }
}

#[component]
pub fn KpiRow(props: KpiRowProps) -> Element {
    rsx! {
        div { class: "kpi-row",
            for tile in props.tiles.iter().cloned() {
                {
                    let value_class = tile.tone.class();
                    rsx! {
                        div { key: "{tile.label}", class: "kpi-tile",
                            div { class: "kpi-tile__label", "{tile.label}" }
                            div { class: "{value_class}", "{tile.value}" }
                        }
                    }
                }
            }
        }
    }
}
