//! Remote Logs view — Dioxus port of
//! `apps/web/components/observability/logs-explorer.tsx`.
//!
//! Fires two requests per "tick" (preset change, search Enter, manual
//! refresh):
//! - `POST /observability/logs/query` for the row data
//! - `GET  /observability/logs/stats`  for the KPI tiles
//!
//! Stats failures don't block rows (KPIs render as `—`), and vice-versa. Both
//! paths run through the typed `OrgClient`, which already does bearer
//! injection + one-shot 401-then-remint at the layer below us.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use dioxus::prelude::*;
use observability_studio_client::api::logs::{
    LogEntry, LogQuery, LogQueryResult, LogStats, TimeRange,
};
use uuid::Uuid;

use crate::components::icons::FileTextIcon;
use crate::components::kpi::{KpiRow, KpiTile, KpiTone};
use crate::components::time_range::{TimePreset, TimeRangePicker};
use crate::persistence::OrgRegistry;
use crate::state::ApiState;

const PAGE_LIMIT: u32 = 100;

/// Bundle of state that triggers a fetch. We hash dependencies via a single
/// signal so `use_resource` re-runs the right number of times.
#[derive(Clone, PartialEq)]
struct FetchKey {
    org_id: Uuid,
    preset: TimePreset,
    search: String,
    nonce: u64,
}

#[component]
pub fn LogsView(org_id: Uuid) -> Element {
    let registry = use_context::<Signal<OrgRegistry>>();
    let api_state = use_context::<Arc<ApiState>>();
    let label = registry()
        .entries
        .iter()
        .find(|e| e.org_id == org_id)
        .map(|e| e.label.clone())
        .unwrap_or_default();

    // Filter state.
    let mut preset = use_signal(TimePreset::default);
    let mut search_draft = use_signal(String::new);
    let mut search_committed = use_signal(String::new);
    let mut refresh_nonce = use_signal(|| 0u64);

    // Expanded-row tracker — keyed by the row's `timestamp + log_stream`,
    // mirroring how the egui port disambiguated CloudWatch rows with the same
    // wall-clock second.
    let mut expanded = use_signal(HashSet::<String>::new);

    // Two parallel async resources so a stats failure doesn't block the table.
    let api_for_q = api_state.clone();
    let logs_resource = use_resource(move || {
        let key = FetchKey {
            org_id,
            preset: preset(),
            search: search_committed(),
            nonce: refresh_nonce(),
        };
        let api_state = api_for_q.clone();
        async move {
            let (start, end) = key.preset.resolve_now();
            let query = LogQuery {
                search: non_empty(&key.search),
                level: None,
                log_group: None,
                function_name: None,
                http_path: None,
                http_status: None,
                trace_id: None,
                time_range: TimeRange { start, end },
                limit: Some(PAGE_LIMIT),
                offset: Some(0),
                order_by: Some("desc".to_string()),
            };
            let api = observability_studio_client::api::ApiClient::new(
                api_state.http.clone(),
                api_state.auth.clone(),
            )
            .ok()?;
            match api.org(org_id).query_logs(&query).await {
                Ok(res) => Some(Ok::<_, String>(res)),
                Err(e) => Some(Err(format!("{e}"))),
            }
        }
    });

    let api_for_s = api_state.clone();
    let stats_resource = use_resource(move || {
        let key = FetchKey {
            org_id,
            preset: preset(),
            search: search_committed(),
            nonce: refresh_nonce(),
        };
        let api_state = api_for_s.clone();
        async move {
            let (start, end) = key.preset.resolve_now();
            let api = observability_studio_client::api::ApiClient::new(
                api_state.http.clone(),
                api_state.auth.clone(),
            )
            .ok()?;
            api.org(org_id).log_stats(start, end).await.ok()
        }
    });

    rsx! {
        header { class: "view-header",
            div { class: "view-header__icon", FileTextIcon {} }
            div { class: "view-header__title-block",
                div { class: "view-header__title", "Logs" }
                div { class: "view-header__sub",
                    "{label} — full-text + facets across CloudWatch via /logs/query"
                }
            }
            div { class: "view-header__actions",
                TimeRangePicker {
                    selected: preset(),
                    on_change: move |p| preset.set(p),
                }
                button {
                    class: "btn btn--outline",
                    onclick: move |_| {
                        refresh_nonce.with_mut(|n| *n += 1);
                    },
                    "Refresh"
                }
            }
        }

        div { class: "toolbar",
            input {
                class: "toolbar__search",
                placeholder: "search messages, levels, function names…",
                value: "{search_draft()}",
                oninput: move |evt| search_draft.set(evt.value()),
                onkeydown: move |evt| {
                    if evt.key() == Key::Enter {
                        search_committed.set(search_draft());
                    }
                },
            }
            div { class: "toolbar__spacer" }
            {render_meta(&logs_resource.read())}
        }

        {render_kpis(&stats_resource.read())}

        {match logs_resource.read().as_ref() {
            None => rsx! { div { class: "logs__loading", "Loading…" } },
            Some(None) => rsx! { div { class: "logs__error", "Could not initialise the API client." } },
            Some(Some(Err(msg))) => rsx! { div { class: "logs__error", "{msg}" } },
            Some(Some(Ok(result))) if result.data.is_empty() => rsx! {
                div { class: "logs__empty", "No log rows match the current filters." }
            },
            Some(Some(Ok(result))) => render_table(result, expanded.read().clone(), move |key| {
                if expanded.read().contains(&key) {
                    expanded.with_mut(|s| { s.remove(&key); });
                } else {
                    expanded.with_mut(|s| { s.insert(key); });
                }
            }),
        }}
    }
}

fn render_meta(state: &Option<Option<Result<LogQueryResult, String>>>) -> Element {
    match state {
        Some(Some(Ok(r))) => {
            // Lift the format!()-able value out of the rsx body — rsx's
            // interpolation parser can't handle nested method calls with
            // escaped quotes.
            let shown = r.data.len();
            let total = r.total;
            let queried = Utc::now().format("%H:%M:%S").to_string();
            rsx! {
                div { class: "toolbar__meta",
                    "{shown} rows · {total} total · queried {queried}"
                }
            }
        }
        _ => rsx! { div { class: "toolbar__meta", "" } },
    }
}

fn render_kpis(state: &Option<Option<LogStats>>) -> Element {
    let stats: Option<LogStats> = match state {
        Some(Some(s)) => Some(s.clone()),
        _ => None,
    };
    let (total, errors, rate, p95, error_tone, rate_tone) = match &stats {
        Some(s) => {
            let total = s.total_logs;
            let errors = s.errors_total();
            let rate = s.error_rate_pct();
            let p95 = s.duration_percentiles.p95;
            let error_tone = if errors > 0 { KpiTone::Destructive } else { KpiTone::Default };
            let rate_tone = if rate > 1.0 { KpiTone::Warning } else { KpiTone::Default };
            (total, errors, rate, p95, error_tone, rate_tone)
        }
        None => (0, 0, 0.0, 0.0, KpiTone::Default, KpiTone::Default),
    };

    let tiles = vec![
        KpiTile {
            label: "Total logs".into(),
            value: if stats.is_some() { thousands(total) } else { "—".into() },
            tone: KpiTone::Default,
        },
        KpiTile {
            label: "Errors".into(),
            value: if stats.is_some() { thousands(errors) } else { "—".into() },
            tone: error_tone,
        },
        KpiTile {
            label: "Error rate".into(),
            value: if stats.is_some() { format!("{rate:.2}%") } else { "—".into() },
            tone: rate_tone,
        },
        KpiTile {
            label: "P95 duration".into(),
            value: if stats.is_some() { format!("{p95:.0} ms") } else { "—".into() },
            tone: KpiTone::Default,
        },
    ];

    rsx! { KpiRow { tiles } }
}

fn render_table(
    result: &LogQueryResult,
    expanded: HashSet<String>,
    on_toggle: impl FnMut(String) + 'static + Clone,
) -> Element {
    rsx! {
        div { class: "logs",
            table { class: "logs__table",
                thead {
                    tr {
                        th { class: "logs__th", "Timestamp" }
                        th { class: "logs__th", "Level" }
                        th { class: "logs__th", "Function" }
                        th { class: "logs__th", "Message" }
                        th { class: "logs__th", "Status" }
                    }
                }
                tbody {
                    for entry in result.data.iter().cloned() {
                        {
                            let key = row_key(&entry);
                            let expanded_now = expanded.contains(&key);
                            let row_class = if expanded_now { "logs__row logs__row--expanded" } else { "logs__row" };
                            let mut on_toggle = on_toggle.clone();
                            let key_for_click = key.clone();
                            rsx! {
                                tr {
                                    key: "{key}",
                                    class: "{row_class}",
                                    onclick: move |_| on_toggle(key_for_click.clone()),
                                    td { class: "logs__td logs__ts", "{format_ts(&entry.timestamp)}" }
                                    td { class: "logs__td",
                                        {render_level(entry.level.as_deref())}
                                    }
                                    td { class: "logs__td logs__fn",
                                        "{entry.function_name.clone().unwrap_or_default()}"
                                    }
                                    td { class: "logs__td logs__msg", "{entry.message}" }
                                    td { class: "logs__td",
                                        {render_status(entry.http_status)}
                                    }
                                }
                                if expanded_now {
                                    tr {
                                        key: "{key}-detail",
                                        td { colspan: 5, class: "logs__detail",
                                            {render_detail(&entry)}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_level(level: Option<&str>) -> Element {
    let raw = level.unwrap_or("").trim();
    if raw.is_empty() {
        return rsx! { span {} };
    }
    let modifier = match raw.to_ascii_lowercase().as_str() {
        "fatal" => "fatal",
        "error" => "error",
        "warn" | "warning" => "warn",
        "info" => "info",
        "debug" => "debug",
        "trace" => "trace",
        _ => "info",
    };
    let class = format!("logs__level logs__level--{modifier}");
    let display = raw.to_ascii_uppercase();
    rsx! { span { class: "{class}", "{display}" } }
}

fn render_status(status: Option<i64>) -> Element {
    let Some(code) = status else { return rsx! { span {} } };
    let bucket = match code {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "2xx",
    };
    let class = format!("logs__status logs__status--{bucket}");
    rsx! { span { class: "{class}", "{code}" } }
}

fn render_detail(entry: &LogEntry) -> Element {
    let rows: Vec<(String, String)> = [
        ("log_group", entry.log_group.clone().unwrap_or_default()),
        ("log_stream", entry.log_stream.clone().unwrap_or_default()),
        ("request_id", entry.request_id.clone().unwrap_or_default()),
        ("trace_id", entry.trace_id.clone().unwrap_or_default()),
        (
            "duration_ms",
            entry.duration_ms.map(|d| format!("{d:.1}")).unwrap_or_default(),
        ),
        (
            "http",
            entry
                .http_method
                .as_deref()
                .zip(entry.http_path.as_deref())
                .map(|(m, p)| format!("{m} {p}"))
                .unwrap_or_default(),
        ),
        ("error", entry.error.clone().unwrap_or_default()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| (k.to_string(), v))
    .collect();

    let parsed: Option<Vec<(String, String)>> = entry.parsed_fields.as_ref().filter(|m| !m.is_empty()).map(|m| {
        let mut v: Vec<_> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    });

    rsx! {
        div { class: "logs__detail-grid",
            for (k, v) in rows.into_iter() {
                {
                    rsx! {
                        div { key: "{k}-k", class: "logs__detail-key", "{k}" }
                        div { key: "{k}-v", class: "logs__detail-val", "{v}" }
                    }
                }
            }
        }
        if let Some(parsed) = parsed {
            div { class: "logs__detail-section", "Parsed fields" }
            div { class: "logs__detail-grid",
                for (k, v) in parsed.into_iter() {
                    {
                        let kk = format!("p-{k}");
                        rsx! {
                            div { key: "{kk}-k", class: "logs__detail-key", "{k}" }
                            div { key: "{kk}-v", class: "logs__detail-val", "{v}" }
                        }
                    }
                }
            }
        }
    }
}

// ---------- pure helpers (tested below) -------------------------------------

fn row_key(entry: &LogEntry) -> String {
    let stream = entry.log_stream.as_deref().unwrap_or("");
    format!("{}|{stream}", entry.timestamp)
}

/// Strip ISO-8601 chrome so dense lists stay scannable. `2026-05-23T19:00:12.234Z`
/// → `2026-05-23 19:00:12`.
fn format_ts(iso: &str) -> String {
    let no_t = iso.replacen('T', " ", 1);
    // Drop fractional seconds + the trailing `Z`.
    let no_frac = no_t.split_once('.').map(|(a, _)| a.to_string()).unwrap_or(no_t);
    no_frac.trim_end_matches('Z').to_string()
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

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_key_combines_timestamp_and_stream() {
        let e = LogEntry {
            timestamp: "2026-05-23T19:00:00Z".into(),
            organization_id: "o".into(),
            aws_account_id: None,
            log_group: None,
            log_stream: Some("stream-abc".into()),
            message: "m".into(),
            level: None,
            request_id: None,
            function_name: None,
            http_method: None,
            http_path: None,
            http_status: None,
            duration_ms: None,
            error: None,
            user_id: None,
            trace_id: None,
            is_json: None,
            parsed_fields: None,
            raw: None,
            ingested_at: None,
        };
        assert_eq!(row_key(&e), "2026-05-23T19:00:00Z|stream-abc");
    }

    #[test]
    fn row_key_handles_missing_stream() {
        let mut e = LogEntry {
            timestamp: "2026-05-23T19:00:00Z".into(),
            organization_id: "o".into(),
            aws_account_id: None,
            log_group: None,
            log_stream: None,
            message: "m".into(),
            level: None,
            request_id: None,
            function_name: None,
            http_method: None,
            http_path: None,
            http_status: None,
            duration_ms: None,
            error: None,
            user_id: None,
            trace_id: None,
            is_json: None,
            parsed_fields: None,
            raw: None,
            ingested_at: None,
        };
        assert_eq!(row_key(&e), "2026-05-23T19:00:00Z|");
        e.log_stream = Some(String::new());
        assert_eq!(row_key(&e), "2026-05-23T19:00:00Z|");
    }

    #[test]
    fn format_ts_strips_chrome_and_fractions() {
        assert_eq!(format_ts("2026-05-23T19:00:12Z"), "2026-05-23 19:00:12");
        assert_eq!(format_ts("2026-05-23T19:00:12.234Z"), "2026-05-23 19:00:12");
        assert_eq!(format_ts("2026-05-23T19:00:12"), "2026-05-23 19:00:12");
    }

    #[test]
    fn thousands_grouping() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(42), "42");
        assert_eq!(thousands(1_234), "1,234");
        assert_eq!(thousands(12_345_678), "12,345,678");
    }

    #[test]
    fn non_empty_trims_and_drops_blank() {
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty("   "), None);
        assert_eq!(non_empty("  hi "), Some("hi".into()));
    }
}
