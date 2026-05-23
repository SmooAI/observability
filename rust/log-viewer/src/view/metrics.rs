//! Remote Metrics view — mirrors `apps/web/components/observability/metrics/metrics-explorer.tsx`.
//!
//! Left rail picks a metric. Right pane renders the time-series in one of
//! three modes — Mean (default), Percentiles (p50/p95/p99), or Heatmap (for
//! histograms). The mode toggle only shows Percentiles + Heatmap when the
//! current metric kind is Histogram, matching the dashboard.

use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};
use uuid::Uuid;

use crate::api::metrics::{
    MetricDescriptor, MetricDescriptorList, MetricHeatmapPoint, MetricKind, MetricListParams,
    MetricPercentilePoint, MetricTimeSeriesPoint, TimeseriesMode, TimeseriesParams,
    TimeseriesPayload,
};
use crate::api::ApiClient;
use crate::widgets::{
    heatmap::{Heatmap, HeatmapPoint},
    time_range::{preset_picker, TimePreset},
};

#[derive(Default)]
pub struct RemoteMetricsView {
    pub org_id: Option<Uuid>,
    preset: TimePreset,
    mode: ViewMode,
    metrics: Vec<MetricDescriptor>,
    metrics_loading: bool,
    metrics_rx: Option<Receiver<MetricsListOutcome>>,
    selected: Option<usize>,
    ts_loading: bool,
    ts_rx: Option<Receiver<TimeseriesOutcome>>,
    ts: Option<TimeseriesPayload>,
    last_error: Option<String>,
    search: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ViewMode {
    #[default]
    Mean,
    Percentiles,
    Heatmap,
}

impl ViewMode {
    fn to_api(self) -> TimeseriesMode {
        match self {
            Self::Mean => TimeseriesMode::Mean,
            Self::Percentiles => TimeseriesMode::Percentiles,
            Self::Heatmap => TimeseriesMode::Heatmap,
        }
    }
}

enum MetricsListOutcome {
    Ok(Vec<MetricDescriptor>),
    Err(String),
}

enum TimeseriesOutcome {
    Ok(TimeseriesPayload),
    Err(String),
}

impl RemoteMetricsView {
    pub fn for_org(org_id: Uuid) -> Self {
        Self {
            org_id: Some(org_id),
            ..Default::default()
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        let Some(org_id) = self.org_id else {
            ui.centered_and_justified(|ui| {
                ui.label("No org selected. Add an org under ⚙ Settings.");
            });
            return;
        };

        self.drain(ui.ctx());

        // Auto-fire metric list once.
        if self.metrics.is_empty() && !self.metrics_loading && self.metrics_rx.is_none() {
            self.fire_metrics(org_id, api, runtime);
        }

        egui::SidePanel::left("metrics_picker")
            .resizable(true)
            .default_width(280.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Metrics");
                    if self.metrics_loading {
                        ui.spinner();
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("filter")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);

                // Snapshot the filtered list so we can iterate without
                // borrowing self for `selected = i` writes.
                let needle = self.search.trim().to_ascii_lowercase();
                let filtered: Vec<(usize, MetricDescriptor)> = self
                    .metrics
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| {
                        needle.is_empty()
                            || m.metric_name.to_ascii_lowercase().contains(&needle)
                    })
                    .map(|(i, m)| (i, m.clone()))
                    .collect();

                let prev_selected = self.selected;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, m) in &filtered {
                        let active = self.selected == Some(*i);
                        let resp = render_metric_row(ui, m, active);
                        if resp.clicked() {
                            self.selected = Some(*i);
                        }
                    }
                });
                if self.selected != prev_selected {
                    // Reset mode if new selection isn't a histogram and mode
                    // was histogram-only.
                    if let Some(idx) = self.selected {
                        if let Some(m) = self.metrics.get(idx) {
                            if !matches!(m.kind, MetricKind::Histogram)
                                && !matches!(self.mode, ViewMode::Mean)
                            {
                                self.mode = ViewMode::Mean;
                            }
                        }
                    }
                    self.fire_timeseries(org_id, api, runtime);
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_main(ui, org_id, api, runtime);
        });
    }

    fn render_main(
        &mut self,
        ui: &mut egui::Ui,
        org_id: Uuid,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        let Some(selected_idx) = self.selected else {
            ui.centered_and_justified(|ui| ui.label("Pick a metric on the left."));
            return;
        };
        let Some(metric) = self.metrics.get(selected_idx).cloned() else {
            return;
        };

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&metric.metric_name).monospace().strong());
            ui.label(
                egui::RichText::new(format!("{:?}", metric.kind))
                    .small()
                    .color(egui::Color32::from_gray(160)),
            );
            if let Some(unit) = &metric.unit {
                ui.label(
                    egui::RichText::new(format!("({unit})"))
                        .small()
                        .color(egui::Color32::from_gray(160)),
                );
            }
            if let Some(svc) = &metric.service_name {
                ui.label(
                    egui::RichText::new(svc)
                        .small()
                        .color(egui::Color32::from_gray(160)),
                );
            }
        });
        ui.add_space(4.0);

        // Mode toggle — only show non-Mean modes for histograms.
        ui.horizontal(|ui| {
            for (label, value, requires_hist) in [
                ("Mean", ViewMode::Mean, false),
                ("Percentiles", ViewMode::Percentiles, true),
                ("Heatmap", ViewMode::Heatmap, true),
            ] {
                let enabled = !requires_hist || matches!(metric.kind, MetricKind::Histogram);
                if ui
                    .add_enabled(enabled, egui::SelectableLabel::new(self.mode == value, label))
                    .clicked()
                {
                    self.mode = value;
                    self.fire_timeseries(org_id, api, runtime);
                }
            }
            ui.separator();
            if preset_picker(ui, &mut self.preset) {
                self.fire_timeseries(org_id, api, runtime);
            }
            if ui.button("⟳ Refresh").clicked() {
                self.fire_timeseries(org_id, api, runtime);
            }
            if self.ts_loading {
                ui.spinner();
            }
        });

        ui.add_space(6.0);
        if let Some(err) = &self.last_error {
            ui.colored_label(egui::Color32::from_rgb(220, 100, 100), err);
            ui.add_space(4.0);
        }

        if self.ts.is_none() && !self.ts_loading && self.ts_rx.is_none() {
            self.fire_timeseries(org_id, api, runtime);
        }

        match self.ts.clone() {
            Some(TimeseriesPayload::Mean { points }) => render_mean(ui, &points),
            Some(TimeseriesPayload::Percentiles { points }) => render_percentiles(ui, &points),
            Some(TimeseriesPayload::Heatmap { points }) => render_heatmap(ui, &points),
            None => {
                ui.centered_and_justified(|ui| ui.label("Loading…"));
            }
        }
    }

    fn fire_metrics(&mut self, org_id: Uuid, api: &ApiClient, runtime: &tokio::runtime::Handle) {
        self.metrics_loading = true;
        let (tx, rx): (Sender<MetricsListOutcome>, _) = std::sync::mpsc::channel();
        self.metrics_rx = Some(rx);
        let api = api.clone();
        runtime.spawn(async move {
            let outcome = match api.org(org_id).list_metrics(&MetricListParams::default()).await {
                Ok(MetricDescriptorList { metrics }) => MetricsListOutcome::Ok(metrics),
                Err(e) => MetricsListOutcome::Err(format!("{e:?}")),
            };
            let _ = tx.send(outcome);
        });
    }

    fn fire_timeseries(
        &mut self,
        org_id: Uuid,
        api: &ApiClient,
        runtime: &tokio::runtime::Handle,
    ) {
        let Some(idx) = self.selected else { return };
        let Some(metric) = self.metrics.get(idx).cloned() else { return };

        let (start, end) = self.preset.resolve_now();
        let params = TimeseriesParams {
            metric_name: metric.metric_name.clone(),
            since_ms: Some(start.timestamp_millis()),
            until_ms: Some(end.timestamp_millis()),
            bucket_ms: None,
            group_by: None,
            environment: None,
            service_name: None,
            mode: Some(self.mode.to_api()),
        };

        self.ts_loading = true;
        self.ts = None;
        let (tx, rx): (Sender<TimeseriesOutcome>, _) = std::sync::mpsc::channel();
        self.ts_rx = Some(rx);
        let api = api.clone();
        runtime.spawn(async move {
            let outcome = match api.org(org_id).metric_timeseries(&params).await {
                Ok(payload) => TimeseriesOutcome::Ok(payload),
                Err(e) => TimeseriesOutcome::Err(format!("{e:?}")),
            };
            let _ = tx.send(outcome);
        });
    }

    fn drain(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.metrics_rx.as_ref() {
            if let Ok(outcome) = rx.try_recv() {
                self.metrics_rx = None;
                self.metrics_loading = false;
                match outcome {
                    MetricsListOutcome::Ok(metrics) => self.metrics = metrics,
                    MetricsListOutcome::Err(e) => self.last_error = Some(e),
                }
                ctx.request_repaint();
            }
        }
        if let Some(rx) = self.ts_rx.as_ref() {
            if let Ok(outcome) = rx.try_recv() {
                self.ts_rx = None;
                self.ts_loading = false;
                match outcome {
                    TimeseriesOutcome::Ok(p) => self.ts = Some(p),
                    TimeseriesOutcome::Err(e) => self.last_error = Some(e),
                }
                ctx.request_repaint();
            }
        }
    }
}

fn render_metric_row(
    ui: &mut egui::Ui,
    metric: &MetricDescriptor,
    active: bool,
) -> egui::Response {
    let label = egui::RichText::new(&metric.metric_name).monospace().small();
    let label = if active { label.strong() } else { label };
    egui::Frame::none()
        .inner_margin(egui::Margin::same(4.0))
        .fill(if active {
            egui::Color32::from_gray(40)
        } else {
            egui::Color32::TRANSPARENT
        })
        .show(ui, |ui| {
            ui.label(label);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:?}", metric.kind))
                        .small()
                        .color(kind_color(metric.kind)),
                );
                ui.label(
                    egui::RichText::new(format!("{} samples", metric.sample_count))
                        .small()
                        .color(egui::Color32::from_gray(150)),
                );
            });
        })
        .response
        .interact(egui::Sense::click())
}

fn kind_color(kind: MetricKind) -> egui::Color32 {
    match kind {
        MetricKind::Counter => crate::theme::smoo::GREEN,
        MetricKind::Gauge => crate::theme::smoo::BLUE_400,
        MetricKind::Histogram => crate::theme::smoo::ORANGE,
    }
}

fn render_mean(ui: &mut egui::Ui, points: &[MetricTimeSeriesPoint]) {
    // Group by `group_key`. Each unique key becomes one line.
    let by_key = group_by_key(points, |p| p.group_key.clone(), |p| {
        // Match the dashboard: render mean = value/count rather than raw sum.
        let v = if p.count == 0 { p.value } else { p.value / p.count as f64 };
        [p.bucket_ms as f64, v]
    });

    Plot::new("metric_mean")
        .height(ui.available_height() - 8.0)
        .show(ui, |plot_ui| {
            for (name, points) in &by_key {
                plot_ui.line(Line::new(PlotPoints::from(points.clone())).name(name));
            }
        });
}

fn render_percentiles(ui: &mut egui::Ui, points: &[MetricPercentilePoint]) {
    let mut p50: Vec<[f64; 2]> = Vec::new();
    let mut p95: Vec<[f64; 2]> = Vec::new();
    let mut p99: Vec<[f64; 2]> = Vec::new();
    for p in points {
        if let Some(v) = p.p50 {
            p50.push([p.bucket_ms as f64, v]);
        }
        if let Some(v) = p.p95 {
            p95.push([p.bucket_ms as f64, v]);
        }
        if let Some(v) = p.p99 {
            p99.push([p.bucket_ms as f64, v]);
        }
    }

    Plot::new("metric_percentiles")
        .height(ui.available_height() - 8.0)
        .legend(egui_plot::Legend::default())
        .show(ui, |plot_ui| {
            plot_ui.line(Line::new(PlotPoints::from(p50)).name("p50").color(crate::theme::smoo::GREEN));
            plot_ui.line(Line::new(PlotPoints::from(p95)).name("p95").color(crate::theme::smoo::ORANGE));
            plot_ui.line(Line::new(PlotPoints::from(p99)).name("p99").color(crate::theme::smoo::RED));
        });
}

fn render_heatmap(ui: &mut egui::Ui, points: &[MetricHeatmapPoint]) {
    let mapped: Vec<HeatmapPoint> = points
        .iter()
        .map(|p| HeatmapPoint {
            bucket_ms: p.bucket_ms,
            counts: p.counts.clone(),
            bounds: p.bounds.clone(),
        })
        .collect();
    Heatmap {
        points: &mapped,
        desired_height: (ui.available_height() - 8.0).max(120.0),
    }
    .ui(ui);
}

/// Groups `xs` by key, producing `Vec<(key, Vec<[x, y]>)>`. Each group's
/// points are sorted by `bucket_ms` so the line chart renders monotonically.
fn group_by_key<T, K, G>(xs: &[T], key: K, point: G) -> Vec<(String, Vec<[f64; 2]>)>
where
    K: Fn(&T) -> String,
    G: Fn(&T) -> [f64; 2],
{
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, Vec<[f64; 2]>> = BTreeMap::new();
    for x in xs {
        map.entry(key(x)).or_default().push(point(x));
    }
    for v in map.values_mut() {
        v.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    }
    map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_by_key_sorts_within_group() {
        let pts = vec![
            MetricTimeSeriesPoint {
                bucket_ms: 200,
                group_key: "a".into(),
                value: 4.0,
                count: 2,
            },
            MetricTimeSeriesPoint {
                bucket_ms: 100,
                group_key: "a".into(),
                value: 2.0,
                count: 1,
            },
            MetricTimeSeriesPoint {
                bucket_ms: 100,
                group_key: "b".into(),
                value: 7.0,
                count: 1,
            },
        ];
        let groups = group_by_key(&pts, |p| p.group_key.clone(), |p| [p.bucket_ms as f64, p.value]);
        assert_eq!(groups.len(), 2);
        // BTreeMap sorts keys alphabetically
        assert_eq!(groups[0].0, "a");
        // Within "a", lowest bucket first
        assert_eq!(groups[0].1[0][0], 100.0);
        assert_eq!(groups[0].1[1][0], 200.0);
    }

    #[test]
    fn kind_color_distinct_per_variant() {
        let c = kind_color(MetricKind::Counter);
        let g = kind_color(MetricKind::Gauge);
        let h = kind_color(MetricKind::Histogram);
        assert!(c != g && g != h && c != h);
    }
}
