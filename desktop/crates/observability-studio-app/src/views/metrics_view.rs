//! Remote Metrics view — Dioxus port of
//! `apps/web/components/observability/metrics/metrics-explorer.tsx`.
//!
//! Left rail: filterable metric list. Right pane: header (name + meta + mode
//! toggle + time-range), then a chart area that renders one of three modes:
//!
//! - **Mean** — inline SVG line chart, one polyline per `group_key`.
//! - **Percentiles** — SVG with three polylines (p50/p95/p99) + legend.
//! - **Heatmap** — CSS grid (matches the dashboard), log-scaled cell tint.
//!
//! The mode toggle disables Percentiles + Heatmap for non-histogram metrics so
//! the UI matches dashboard semantics.

use std::sync::Arc;

use dioxus::prelude::*;
use observability_studio_client::api::metrics::{
    MetricDescriptor, MetricDescriptorList, MetricHeatmapPoint, MetricKind, MetricListParams,
    MetricPercentilePoint, MetricTimeSeriesPoint, TimeseriesMode, TimeseriesParams,
    TimeseriesPayload,
};
use observability_studio_client::api::ApiClient;
use uuid::Uuid;

use crate::components::icons::ActivityIcon;
use crate::components::time_range::{TimePreset, TimeRangePicker};
use crate::persistence::OrgRegistry;
use crate::state::ApiState;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Mean,
    Percentiles,
    Heatmap,
}

impl Mode {
    fn to_api(self) -> TimeseriesMode {
        match self {
            Self::Mean => TimeseriesMode::Mean,
            Self::Percentiles => TimeseriesMode::Percentiles,
            Self::Heatmap => TimeseriesMode::Heatmap,
        }
    }
}

fn make_client(api_state: &Arc<ApiState>) -> Option<ApiClient> {
    ApiClient::new(api_state.http.clone(), api_state.auth.clone()).ok()
}

#[component]
pub fn MetricsView(org_id: Uuid) -> Element {
    let registry = use_context::<Signal<OrgRegistry>>();
    let api_state = use_context::<Arc<ApiState>>();
    let label = registry()
        .entries
        .iter()
        .find(|e| e.org_id == org_id)
        .map(|e| e.label.clone())
        .unwrap_or_default();

    let filter = use_signal(String::new);
    let selected_name = use_signal::<Option<String>>(|| None);
    let mode = use_signal(Mode::default);
    let preset = use_signal(TimePreset::default);

    // Metric list — fired once per org.
    let api_for_list = api_state.clone();
    let list_resource = use_resource(move || {
        let api_state = api_for_list.clone();
        async move {
            let api = make_client(&api_state)?;
            match api.org(org_id).list_metrics(&MetricListParams::default()).await {
                Ok(MetricDescriptorList { metrics }) => Some(Ok::<_, String>(metrics)),
                Err(e) => Some(Err(format!("{e}"))),
            }
        }
    });

    // Resolve the active metric descriptor for the right pane.
    let active_descriptor: Option<MetricDescriptor> = match list_resource.read().as_ref() {
        Some(Some(Ok(metrics))) => match selected_name() {
            Some(name) => metrics.iter().find(|m| m.metric_name == name).cloned(),
            None => metrics.first().cloned(),
        },
        _ => None,
    };

    // Timeseries — depends on the active descriptor + mode + preset.
    let api_for_ts = api_state.clone();
    let active_for_ts = active_descriptor.clone();
    let ts_resource = use_resource(move || {
        let api_state = api_for_ts.clone();
        let active = active_for_ts.clone();
        let mode = mode();
        let preset = preset();
        async move {
            let active = active?;
            let (start, end) = preset.resolve_now();
            let params = TimeseriesParams {
                metric_name: active.metric_name.clone(),
                since_ms: Some(start.timestamp_millis()),
                until_ms: Some(end.timestamp_millis()),
                bucket_ms: None,
                group_by: None,
                environment: None,
                service_name: None,
                mode: Some(mode.to_api()),
            };
            let api = make_client(&api_state)?;
            match api.org(org_id).metric_timeseries(&params).await {
                Ok(p) => Some(Ok::<_, String>(p)),
                Err(e) => Some(Err(format!("{e}"))),
            }
        }
    });

    rsx! {
        header { class: "view-header",
            div { class: "view-header__icon", ActivityIcon {} }
            div { class: "view-header__title-block",
                div { class: "view-header__title", "Metrics" }
                div { class: "view-header__sub",
                    "{label} — counters, gauges, histograms + latency heatmap"
                }
            }
        }
        div { class: "metrics",
            {render_picker(&list_resource.read(), selected_name, filter)}
            {render_pane(active_descriptor.as_ref(), &ts_resource.read(), mode, preset)}
        }
    }
}

// ---------- picker rail -----------------------------------------------------

fn render_picker(
    state: &Option<Option<Result<Vec<MetricDescriptor>, String>>>,
    mut selected_name: Signal<Option<String>>,
    mut filter: Signal<String>,
) -> Element {
    let filter_v = filter();
    let needle = filter_v.trim().to_ascii_lowercase();
    rsx! {
        aside { class: "metrics__picker",
            div { class: "metrics__picker-head",
                div { class: "metrics__picker-title", "Metrics" }
                input {
                    class: "metrics__picker-filter",
                    placeholder: "filter…",
                    value: "{filter_v}",
                    oninput: move |evt| filter.set(evt.value()),
                }
            }
            div { class: "metrics__picker-list",
                {match state {
                    None => rsx! { div { class: "logs__loading", "Loading…" } },
                    Some(None) => rsx! { div { class: "logs__error", "Could not initialise the API client." } },
                    Some(Some(Err(msg))) => rsx! { div { class: "logs__error", "{msg}" } },
                    Some(Some(Ok(metrics))) => rsx! {
                        for m in metrics.iter().filter(|m| {
                            needle.is_empty() || m.metric_name.to_ascii_lowercase().contains(&needle)
                        }).cloned() {
                            {
                                let active = selected_name().as_deref() == Some(m.metric_name.as_str());
                                let class = if active { "metric-row metric-row--active" } else { "metric-row" };
                                let key = m.metric_name.clone();
                                let click_name = m.metric_name.clone();
                                let kind_class = format!("badge badge--kind-{}", m.kind.label());
                                let kind_text = m.kind.label();
                                let svc = m.service_name.clone().unwrap_or_default();
                                let samples = thousands(m.sample_count);
                                rsx! {
                                    div {
                                        key: "{key}",
                                        class: "{class}",
                                        onclick: move |_| selected_name.set(Some(click_name.clone())),
                                        div { class: "metric-row__name", "{m.metric_name}" }
                                        div { class: "metric-row__meta",
                                            span { class: "{kind_class}", "{kind_text}" }
                                            if !svc.is_empty() { span { "{svc}" } }
                                            span { class: "metric-row__count", "{samples}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }}
            }
        }
    }
}

// ---------- chart pane ------------------------------------------------------

fn render_pane(
    active: Option<&MetricDescriptor>,
    ts_state: &Option<Option<Result<TimeseriesPayload, String>>>,
    mut mode: Signal<Mode>,
    mut preset: Signal<TimePreset>,
) -> Element {
    let Some(active) = active else {
        return rsx! {
            div { class: "metrics__pane",
                div { class: "metrics__empty", "Pick a metric on the left." }
            }
        };
    };

    let is_histogram = active.kind == MetricKind::Histogram;
    let unit = active.unit.clone().unwrap_or_default();
    let svc = active.service_name.clone().unwrap_or_default();
    let active_name = active.metric_name.clone();
    let kind_class = format!("badge badge--kind-{}", active.kind.label());
    let kind_label = active.kind.label();

    let chart: Element = match ts_state.as_ref() {
        None => rsx! { div { class: "logs__loading", "Loading…" } },
        Some(None) => rsx! { div { class: "logs__error", "Could not initialise the API client." } },
        Some(Some(Err(msg))) => rsx! { div { class: "logs__error", "{msg}" } },
        Some(Some(Ok(TimeseriesPayload::Mean { points }))) => render_mean(points),
        Some(Some(Ok(TimeseriesPayload::Percentiles { points }))) => render_percentiles(points),
        Some(Some(Ok(TimeseriesPayload::Heatmap { points }))) => render_heatmap(points),
    };

    rsx! {
        div { class: "metrics__pane",
            div { class: "metrics__pane-head",
                span { class: "metrics__pane-name", "{active_name}" }
                span { class: "{kind_class}", "{kind_label}" }
                if !unit.is_empty() {
                    span { class: "metrics__pane-meta", "{unit}" }
                }
                if !svc.is_empty() {
                    span { class: "metrics__pane-meta", "{svc}" }
                }
                div { style: "flex: 1;" }
                TimeRangePicker {
                    selected: preset(),
                    on_change: move |p| preset.set(p),
                }
                div { class: "metrics__mode",
                    for (m_value, m_label) in [
                        (Mode::Mean, "Mean"),
                        (Mode::Percentiles, "Percentiles"),
                        (Mode::Heatmap, "Heatmap"),
                    ] {
                        {
                            let requires_hist = matches!(m_value, Mode::Percentiles | Mode::Heatmap);
                            let enabled = !requires_hist || is_histogram;
                            let active = m_value == mode();
                            let mut class = String::from("metrics__mode-btn");
                            if active {
                                class.push_str(" metrics__mode-btn--active");
                            }
                            rsx! {
                                button {
                                    key: "{m_label}",
                                    class: "{class}",
                                    disabled: !enabled,
                                    onclick: move |_| mode.set(m_value),
                                    "{m_label}"
                                }
                            }
                        }
                    }
                }
            }
            div { class: "metrics__chart-area",
                {chart}
            }
        }
    }
}

// ---------- chart renderers -------------------------------------------------

fn render_mean(points: &[MetricTimeSeriesPoint]) -> Element {
    let series = group_by_key(points, |p| p.group_key.clone(), |p| {
        let v = if p.count == 0 { p.value } else { p.value / p.count as f64 };
        (p.bucket_ms, v)
    });
    rsx! {
        {render_line_chart(&series, &["mean"])}
    }
}

fn render_percentiles(points: &[MetricPercentilePoint]) -> Element {
    // Materialise three series: p50, p95, p99.
    let mut p50: Vec<(i64, f64)> = Vec::new();
    let mut p95: Vec<(i64, f64)> = Vec::new();
    let mut p99: Vec<(i64, f64)> = Vec::new();
    for p in points {
        if let Some(v) = p.p50 {
            p50.push((p.bucket_ms, v));
        }
        if let Some(v) = p.p95 {
            p95.push((p.bucket_ms, v));
        }
        if let Some(v) = p.p99 {
            p99.push((p.bucket_ms, v));
        }
    }
    p50.sort_by_key(|p| p.0);
    p95.sort_by_key(|p| p.0);
    p99.sort_by_key(|p| p.0);
    let series = vec![
        ("p50".to_string(), p50),
        ("p95".to_string(), p95),
        ("p99".to_string(), p99),
    ];
    rsx! {
        {render_line_chart(&series, &["p50", "p95", "p99"])}
    }
}

/// Render N polylines onto a shared 1000x320 SVG canvas. `series` is a list of
/// (label, points); `line_class_suffixes` map each line to one of the
/// `.chart-svg__line--{p50,p95,p99,mean}` style modifiers (cycled if needed).
fn render_line_chart(
    series: &[(String, Vec<(i64, f64)>)],
    line_class_suffixes: &[&str],
) -> Element {
    let (min_x, max_x, min_y, max_y) = bounds(series);
    if max_x == min_x || max_y == min_y || series.iter().all(|(_, pts)| pts.is_empty()) {
        return rsx! { div { class: "metrics__empty", "No data points for this range." } };
    }

    let w = 1000.0_f64;
    let h = 320.0_f64;
    let pad_l = 44.0_f64;
    let pad_r = 12.0_f64;
    let pad_t = 12.0_f64;
    let pad_b = 22.0_f64;
    let inner_w = w - pad_l - pad_r;
    let inner_h = h - pad_t - pad_b;

    let xnorm = move |x: i64| (x - min_x) as f64 / (max_x - min_x) as f64;
    let ynorm = move |y: f64| (y - min_y) / (max_y - min_y);

    // Polyline `points` strings — one per series.
    let polylines: Vec<(String, String, &str)> = series
        .iter()
        .enumerate()
        .map(|(i, (label, pts))| {
            let suffix = line_class_suffixes
                .get(i)
                .copied()
                .or_else(|| line_class_suffixes.last().copied())
                .unwrap_or("mean");
            let pts_str: String = pts
                .iter()
                .map(|(x, y)| {
                    let px = pad_l + xnorm(*x) * inner_w;
                    let py = pad_t + (1.0 - ynorm(*y)) * inner_h;
                    format!("{px:.1},{py:.1}")
                })
                .collect::<Vec<_>>()
                .join(" ");
            (label.clone(), pts_str, suffix)
        })
        .collect();

    // Sparse horizontal grid lines (4 ticks).
    let mut grid_lines: Vec<(f64, String)> = Vec::new();
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let y = pad_t + (1.0 - frac) * inner_h;
        let val = min_y + frac * (max_y - min_y);
        grid_lines.push((y, format_axis(val)));
    }

    let x_left = format_ms(min_x);
    let x_right = format_ms(max_x);

    let viewbox = format!("0 0 {w} {h}");

    rsx! {
        svg {
            class: "chart-svg",
            view_box: "{viewbox}",
            preserve_aspect_ratio: "none",
            // Grid lines + Y labels
            for (gy, label) in grid_lines.into_iter() {
                {
                    let line_x1 = format!("{pad_l:.1}");
                    let line_x2 = format!("{:.1}", w - pad_r);
                    let line_y = format!("{gy:.1}");
                    let text_y = format!("{:.1}", gy + 3.5);
                    rsx! {
                        line {
                            class: "chart-svg__grid",
                            x1: "{line_x1}", x2: "{line_x2}",
                            y1: "{line_y}", y2: "{line_y}",
                        }
                        text {
                            class: "chart-svg__axis-label",
                            x: "4", y: "{text_y}",
                            "{label}"
                        }
                    }
                }
            }
            // X axis endpoints
            {
                let x_left_x = format!("{pad_l:.1}");
                let x_right_x = format!("{:.1}", w - pad_r);
                let axis_y = format!("{:.1}", h - 6.0);
                rsx! {
                    text { class: "chart-svg__axis-label", x: "{x_left_x}", y: "{axis_y}",
                        "{x_left}"
                    }
                    text { class: "chart-svg__axis-label", x: "{x_right_x}", y: "{axis_y}", text_anchor: "end",
                        "{x_right}"
                    }
                }
            }
            // Polylines
            for (label, pts, suffix) in polylines.iter() {
                {
                    let class = format!("chart-svg__line chart-svg__line--{suffix}");
                    rsx! {
                        polyline {
                            key: "{label}",
                            class: "{class}",
                            points: "{pts}",
                        }
                    }
                }
            }
        }
        // Legend
        {render_legend(line_class_suffixes, series)}
    }
}

fn render_legend(suffixes: &[&str], series: &[(String, Vec<(i64, f64)>)]) -> Element {
    rsx! {
        div { class: "chart-legend",
            for (i, (label, _)) in series.iter().enumerate() {
                {
                    let suffix = suffixes.get(i).copied().unwrap_or("mean");
                    let color_var = match suffix {
                        "p50" => "oklch(0.657 0.112 194.8)",
                        "p95" => "oklch(0.769 0.164 71)",
                        "p99" => "oklch(0.712 0.181 22.4)",
                        _ => "oklch(0.725 0.102 233.4)",
                    };
                    let style = format!("background:{color_var};");
                    rsx! {
                        span { key: "{label}",
                            span { class: "chart-legend__swatch", style: "{style}" }
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}

fn render_heatmap(points: &[MetricHeatmapPoint]) -> Element {
    if points.is_empty() {
        return rsx! { div { class: "metrics__empty", "No data points for this range." } };
    }

    // Establish row count from the first non-empty bounds vec.
    let bounds = points
        .iter()
        .find(|p| !p.bounds.is_empty())
        .map(|p| p.bounds.clone())
        .unwrap_or_default();
    if bounds.is_empty() {
        return rsx! { div { class: "metrics__empty", "Histogram has no buckets in this range." } };
    }
    let rows = bounds.len() + 1; // +1 for +∞ overflow
    let cols = points.len();
    let max_count: u64 = points
        .iter()
        .flat_map(|p| p.counts.iter().copied())
        .max()
        .unwrap_or(0)
        .max(1);

    let grid_style = format!(
        "grid-template-columns: repeat({cols}, 1fr); grid-template-rows: repeat({rows}, 1fr); aspect-ratio: {ratio};",
        cols = cols,
        rows = rows,
        ratio = format!("{cols} / {rows}"),
    );

    let xa = format_ms(points.first().unwrap().bucket_ms);
    let xb = format_ms(points.last().unwrap().bucket_ms);

    rsx! {
        div { class: "heatmap", style: "{grid_style}",
            // Row 0 visually is the TOP — show "+∞" overflow. So we iterate
            // top-down: row index = rows - 1 (overflow) down to 0 (lowest).
            for row in (0..rows).rev() {
                for (col_idx, point) in points.iter().enumerate() {
                    {
                        let count = point.counts.get(row).copied().unwrap_or(0);
                        let lightness = log_scaled_lightness(count, max_count);
                        let style = format!(
                            "background: oklch({lightness:.3} 0.13 195);"
                        );
                        let bucket_label = if row == bounds.len() {
                            "≥ +∞".to_string()
                        } else if row == 0 {
                            format!("< {}", format_bound(bounds[0]))
                        } else {
                            let lo = bounds[row - 1];
                            let hi = bounds[row];
                            format!("{} – {}", format_bound(lo), format_bound(hi))
                        };
                        let title = format!("{} · {bucket_label} · {count} samples", format_ms(point.bucket_ms));
                        let key = format!("{}-{}", row, col_idx);
                        rsx! {
                            div {
                                key: "{key}",
                                class: "heatmap__cell",
                                style: "{style}",
                                title: "{title}",
                            }
                        }
                    }
                }
            }
        }
        div { class: "heatmap__axis-x",
            span { "{xa}" }
            span { "{xb}" }
        }
    }
}

// ---------- helpers ---------------------------------------------------------

fn group_by_key<T, K, F>(xs: &[T], key: K, point: F) -> Vec<(String, Vec<(i64, f64)>)>
where
    K: Fn(&T) -> String,
    F: Fn(&T) -> (i64, f64),
{
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();
    for x in xs {
        map.entry(key(x)).or_default().push(point(x));
    }
    for v in map.values_mut() {
        v.sort_by_key(|p| p.0);
    }
    map.into_iter().collect()
}

fn bounds(series: &[(String, Vec<(i64, f64)>)]) -> (i64, i64, f64, f64) {
    let mut min_x = i64::MAX;
    let mut max_x = i64::MIN;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (_, pts) in series {
        for (x, y) in pts {
            if *x < min_x { min_x = *x; }
            if *x > max_x { max_x = *x; }
            if *y < min_y { min_y = *y; }
            if *y > max_y { max_y = *y; }
        }
    }
    if min_x == i64::MAX {
        // No data — collapse to 0..1 / 0..1 so callers can render an empty
        // chart frame without dividing by zero.
        return (0, 1, 0.0, 1.0);
    }
    // Pad the y range slightly so the line doesn't kiss the top/bottom.
    let pad = (max_y - min_y).abs() * 0.05;
    let pad = if pad == 0.0 { 1.0 } else { pad };
    (min_x, max_x, min_y - pad, max_y + pad)
}

/// Log-scaled lightness for heatmap cells, in OKLCH lightness units. Empty
/// cells stay near the background (~0.13), heavy cells climb toward 0.7.
fn log_scaled_lightness(count: u64, max_count: u64) -> f64 {
    if count == 0 || max_count == 0 {
        return 0.13;
    }
    let intensity = (count as f64).ln_1p() / (max_count as f64).ln_1p();
    let intensity = intensity.clamp(0.0, 1.0);
    0.13 + intensity * 0.55 // 0.13 → 0.68
}

fn format_axis(v: f64) -> String {
    if v >= 1_000.0 {
        format!("{:.1}k", v / 1_000.0)
    } else if v >= 1.0 {
        format!("{v:.0}")
    } else if v >= 0.01 {
        format!("{v:.2}")
    } else {
        format!("{v:.3}")
    }
}

fn format_bound(b: f64) -> String {
    if b >= 1_000.0 {
        format!("{:.1}k", b / 1_000.0)
    } else if b >= 1.0 {
        format!("{b:.0}")
    } else {
        format!("{b:.3}")
    }
}

fn format_ms(ms: i64) -> String {
    let secs = ms / 1_000;
    let mins = (secs / 60) % 60;
    let hours = (secs / 3_600) % 24;
    format!("{hours:02}:{mins:02}")
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i != 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_by_key_sorts_within_group() {
        let pts = vec![
            MetricTimeSeriesPoint { bucket_ms: 200, group_key: "a".into(), value: 4.0, count: 2 },
            MetricTimeSeriesPoint { bucket_ms: 100, group_key: "a".into(), value: 2.0, count: 1 },
            MetricTimeSeriesPoint { bucket_ms: 100, group_key: "b".into(), value: 7.0, count: 1 },
        ];
        let series = group_by_key(&pts, |p| p.group_key.clone(), |p| (p.bucket_ms, p.value));
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].0, "a");
        assert_eq!(series[0].1[0].0, 100);
        assert_eq!(series[0].1[1].0, 200);
    }

    #[test]
    fn log_scaled_lightness_extremes() {
        // Empty cell sits at the panel background.
        assert!((log_scaled_lightness(0, 0) - 0.13).abs() < 1e-6);
        assert!((log_scaled_lightness(0, 100) - 0.13).abs() < 1e-6);
        // Peak cell is significantly brighter than the empty baseline.
        let peak = log_scaled_lightness(1000, 1000);
        assert!(peak > 0.6, "peak cell should be > 0.6 (got {peak})");
    }

    #[test]
    fn format_bound_buckets() {
        assert_eq!(format_bound(0.005), "0.005");
        assert_eq!(format_bound(42.0), "42");
        assert_eq!(format_bound(1500.0), "1.5k");
    }

    #[test]
    fn bounds_collapses_on_empty_input() {
        let (min_x, max_x, min_y, max_y) = bounds(&[]);
        assert_eq!(min_x, 0);
        assert_eq!(max_x, 1);
        assert_eq!(min_y, 0.0);
        assert_eq!(max_y, 1.0);
    }
}
