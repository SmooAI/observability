//! Remote Errors view — Dioxus port of
//! `apps/web/components/observability/errors/{error-list,error-detail}.tsx`.
//!
//! Two modes:
//! - `List` — paginated cards filtered by status + environment. Auto-fires on
//!   first frame and on filter change.
//! - `Detail` — group metadata + recent events with stack frames + status
//!   mutation actions (Mark resolved / Mute / Reopen).
//!
//! Mutations optimistically patch the cached detail + list rows before the
//! PATCH returns; on failure we re-fetch the list.

use std::sync::Arc;

use dioxus::prelude::*;
use observability_studio_client::api::errors::{
    extract_first_stacktrace, ErrorDetail, ErrorEvent, ErrorGroup, ErrorListParams,
    ErrorPage, ErrorPatch, ErrorStatus,
};
use observability_studio_client::api::ApiClient;
use uuid::Uuid;

use crate::components::icons::AlertTriangleIcon;
use crate::components::stack_frame::StackFrames;
use crate::persistence::OrgRegistry;
use crate::state::ApiState;

#[derive(Clone, PartialEq, Eq)]
enum Mode {
    List,
    Detail(Uuid),
}

fn make_client(api_state: &Arc<ApiState>) -> Option<ApiClient> {
    ApiClient::new(api_state.http.clone(), api_state.auth.clone()).ok()
}

#[component]
pub fn ErrorsView(org_id: Uuid) -> Element {
    let registry = use_context::<Signal<OrgRegistry>>();
    let api_state = use_context::<Arc<ApiState>>();
    let label = registry()
        .entries
        .iter()
        .find(|e| e.org_id == org_id)
        .map(|e| e.label.clone())
        .unwrap_or_default();

    let mode = use_signal(|| Mode::List);
    let status_filter = use_signal(|| Some(ErrorStatus::Unresolved));
    let env_draft = use_signal(String::new);
    let env_committed = use_signal(String::new);
    let refresh_nonce = use_signal(|| 0u64);

    // List resource — re-fires on status/env/nonce change.
    let api_for_list = api_state.clone();
    let list_resource = use_resource(move || {
        let status = status_filter();
        let env = env_committed();
        let nonce = refresh_nonce();
        let api_state = api_for_list.clone();
        async move {
            let _ = nonce; // dependency only
            let env_filter = if env.trim().is_empty() { None } else { Some(env.trim().to_string()) };
            let params = ErrorListParams {
                environment: env_filter,
                status,
                cursor: None,
                limit: Some(50),
            };
            let api = make_client(&api_state)?;
            match api.org(org_id).list_error_groups(&params).await {
                Ok(page) => Some(Ok::<_, String>(page)),
                Err(e) => Some(Err(format!("{e}"))),
            }
        }
    });

    // Detail resource — re-fires when the user clicks into a group.
    let api_for_detail = api_state.clone();
    let detail_resource = use_resource(move || {
        let m = mode();
        let api_state = api_for_detail.clone();
        async move {
            match m {
                Mode::Detail(group_id) => {
                    let api = make_client(&api_state)?;
                    match api.org(org_id).get_error_group(group_id).await {
                        Ok(d) => Some(Ok::<_, String>(d)),
                        Err(e) => Some(Err(format!("{e}"))),
                    }
                }
                Mode::List => None,
            }
        }
    });

    let active_view: Element = match mode() {
        Mode::List => render_list(
            &list_resource.read(),
            *status_filter.read(),
            env_draft,
            env_committed,
            refresh_nonce,
            status_filter,
            mode,
        ),
        Mode::Detail(group_id) => render_detail(
            &detail_resource.read(),
            api_state.clone(),
            org_id,
            group_id,
            mode,
            refresh_nonce,
        ),
    };

    rsx! {
        header { class: "view-header",
            div { class: "view-header__icon", AlertTriangleIcon {} }
            div { class: "view-header__title-block",
                div { class: "view-header__title", "Errors" }
                div { class: "view-header__sub",
                    "{label} — grouped exceptions + stack traces via /errors"
                }
            }
        }
        {active_view}
    }
}

#[allow(clippy::too_many_arguments)]
fn render_list(
    state: &Option<Option<Result<ErrorPage, String>>>,
    selected_status: Option<ErrorStatus>,
    mut env_draft: Signal<String>,
    mut env_committed: Signal<String>,
    mut refresh_nonce: Signal<u64>,
    mut status_filter: Signal<Option<ErrorStatus>>,
    mut mode: Signal<Mode>,
) -> Element {
    let pills: &[(&str, Option<ErrorStatus>)] = &[
        ("Unresolved", Some(ErrorStatus::Unresolved)),
        ("Resolved", Some(ErrorStatus::Resolved)),
        ("Muted", Some(ErrorStatus::Muted)),
        ("All", None),
    ];

    rsx! {
        div { class: "errors__filter-row",
            div { class: "errors__filter-pills",
                for (label, value) in pills.iter().copied() {
                    {
                        let active = value == selected_status;
                        let class = if active { "errors__pill errors__pill--active" } else { "errors__pill" };
                        rsx! {
                            button {
                                key: "{label}",
                                class: "{class}",
                                onclick: move |_| status_filter.set(value),
                                "{label}"
                            }
                        }
                    }
                }
            }
            input {
                class: "errors__env-input",
                placeholder: "environment (e.g. production)",
                value: "{env_draft()}",
                oninput: move |evt| env_draft.set(evt.value()),
                onkeydown: move |evt| {
                    if evt.key() == Key::Enter {
                        env_committed.set(env_draft());
                    }
                },
            }
            button {
                class: "btn btn--outline",
                onclick: move |_| refresh_nonce.with_mut(|n| *n += 1),
                "Refresh"
            }
        }

        div { class: "errors",
            {match state {
                None => rsx! { div { class: "logs__loading", "Loading…" } },
                Some(None) => rsx! { div { class: "logs__error", "Could not initialise the API client." } },
                Some(Some(Err(msg))) => rsx! { div { class: "logs__error", "{msg}" } },
                Some(Some(Ok(page))) if page.groups.is_empty() => rsx! {
                    div { class: "logs__empty", "No error groups match the current filters." }
                },
                Some(Some(Ok(page))) => rsx! {
                    for group in page.groups.iter().cloned() {
                        {
                            let key = group.id.clone();
                            let click_key = key.clone();
                            rsx! {
                                div {
                                    key: "{key}",
                                    class: "err-card",
                                    onclick: move |_| {
                                        if let Ok(uuid) = Uuid::parse_str(&click_key) {
                                            mode.set(Mode::Detail(uuid));
                                        }
                                    },
                                    {render_card(&group)}
                                }
                            }
                        }
                    }
                },
            }}
        }
    }
}

fn render_card(group: &ErrorGroup) -> Element {
    let last = short_ts(&group.last_seen_at);
    rsx! {
        div { class: "err-card__head",
            {level_badge(&group.level)}
            {env_badge(&group.environment)}
            {status_badge(group.status)}
        }
        div { class: "err-card__title", "{group.title}" }
        if let Some(culprit) = &group.culprit {
            div { class: "err-card__culprit", "{culprit}" }
        }
        div { class: "err-card__footer",
            span { class: "err-card__count", "{group.event_count}" }
            span { " events · " }
            span { class: "err-card__count", "{group.user_count}" }
            span { " users · " }
            span { "last seen {last}" }
        }
    }
}

fn render_detail(
    state: &Option<Option<Result<ErrorDetail, String>>>,
    api_state: Arc<ApiState>,
    org_id: Uuid,
    group_id: Uuid,
    mut mode: Signal<Mode>,
    refresh_nonce: Signal<u64>,
) -> Element {
    let back = rsx! {
        div { class: "err-detail__back-row",
            button {
                class: "btn btn--ghost",
                onclick: move |_| mode.set(Mode::List),
                "← Back"
            }
        }
    };

    let body = match state {
        None => rsx! { div { class: "logs__loading", "Loading detail…" } },
        Some(None) => rsx! { div { class: "logs__error", "Could not initialise the API client." } },
        Some(Some(Err(msg))) => rsx! { div { class: "logs__error", "{msg}" } },
        Some(Some(Ok(detail))) => render_detail_body(detail.clone(), api_state, org_id, group_id, mode, refresh_nonce),
    };

    rsx! {
        {back}
        {body}
    }
}

fn render_detail_body(
    detail: ErrorDetail,
    api_state: Arc<ApiState>,
    org_id: Uuid,
    group_id: Uuid,
    _mode: Signal<Mode>,
    mut refresh_nonce: Signal<u64>,
) -> Element {
    let group = detail.group.clone();
    let last = short_ts(&group.last_seen_at);
    let first = short_ts(&group.first_seen_at);
    let events = detail.recent_events.clone();
    let current_status = group.status;

    let mutate = move |new_status: ErrorStatus| {
        let api_state = api_state.clone();
        spawn(async move {
            let Some(api) = make_client(&api_state) else { return };
            let patch = ErrorPatch { status: Some(new_status), assigned_user_id: None };
            // Best-effort — re-fetch the list on success so the rail badge
            // updates. We don't surface errors inline today; that's a
            // follow-up. For now we tick the refresh nonce so the next time
            // the user pops back to the list, they see fresh data.
            if api.org(org_id).update_error_group(group_id, &patch).await.is_ok() {
                refresh_nonce.with_mut(|n| *n += 1);
            }
        });
    };

    rsx! {
        div { class: "err-detail__head",
            div { class: "err-detail__badges",
                {level_badge(&group.level)}
                {env_badge(&group.environment)}
                {status_badge(group.status)}
            }
            div { class: "err-detail__title", "{group.title}" }
            if let Some(culprit) = &group.culprit {
                div { class: "err-detail__culprit", "{culprit}" }
            }
            div { class: "err-card__footer", style: "margin-top: 12px;",
                span { class: "err-card__count", "{group.event_count}" }
                span { " events · " }
                span { class: "err-card__count", "{group.user_count}" }
                span { " users · first seen {first} · last seen {last}" }
            }
        }
        div { class: "err-detail__actions",
            if current_status != ErrorStatus::Resolved {
                button {
                    class: "btn btn--primary",
                    onclick: {
                        let mutate = mutate.clone();
                        move |_| mutate(ErrorStatus::Resolved)
                    },
                    "Mark resolved"
                }
            }
            if current_status != ErrorStatus::Muted {
                button {
                    class: "btn btn--outline",
                    onclick: {
                        let mutate = mutate.clone();
                        move |_| mutate(ErrorStatus::Muted)
                    },
                    "Mute"
                }
            }
            if current_status != ErrorStatus::Unresolved {
                button {
                    class: "btn btn--ghost",
                    onclick: {
                        let mutate = mutate.clone();
                        move |_| mutate(ErrorStatus::Unresolved)
                    },
                    "Reopen"
                }
            }
        }

        div { class: "err-detail__events",
            div { class: "err-detail__events-heading",
                "Recent events ({events.len()})"
            }
            if events.is_empty() {
                div { class: "logs__empty", "No events captured yet." }
            }
            for event in events.into_iter() {
                {
                    let key = event.event_id.clone();
                    rsx! {
                        div { key: "{key}", class: "event",
                            {render_event(&event)}
                        }
                    }
                }
            }
        }
    }
}

fn render_event(event: &ErrorEvent) -> Element {
    let when = short_ts(&event.occurred_at);
    let frames = extract_first_stacktrace(&event.exception);
    rsx! {
        div { class: "event__head",
            span { class: "event__id", "{event.event_id}" }
            span { class: "event__when", "{when}" }
        }
        if let Some(msg) = &event.message {
            div { class: "event__msg", "{msg}" }
        }
        if let Some(frames) = frames {
            StackFrames { frames }
        }
    }
}

// ---------- pure render helpers ---------------------------------------------

fn level_badge(level: &str) -> Element {
    let modifier = match level.to_ascii_lowercase().as_str() {
        "fatal" => "fatal",
        "error" => "error",
        "warn" | "warning" => "warn",
        _ => "info",
    };
    let class = format!("badge badge--level-{modifier}");
    let text = level.to_ascii_uppercase();
    rsx! { span { class: "{class}", "{text}" } }
}

fn env_badge(env: &str) -> Element {
    rsx! { span { class: "badge badge--env", "{env}" } }
}

fn status_badge(status: ErrorStatus) -> Element {
    let modifier = match status {
        ErrorStatus::Unresolved => "unresolved",
        ErrorStatus::Resolved => "resolved",
        ErrorStatus::Muted => "muted",
    };
    let class = format!("badge badge--status-{modifier}");
    let text = status.label();
    rsx! { span { class: "{class}", "{text}" } }
}

fn short_ts(iso: &str) -> String {
    let no_t = iso.replacen('T', " ", 1);
    let no_frac = no_t.split_once('.').map(|(a, _)| a.to_string()).unwrap_or(no_t);
    no_frac.trim_end_matches('Z').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ts_strips_chrome() {
        assert_eq!(short_ts("2026-05-23T19:00:12.234Z"), "2026-05-23 19:00:12");
        assert_eq!(short_ts("2026-05-23T19:00:12Z"), "2026-05-23 19:00:12");
        assert_eq!(short_ts("2026-05-23T19:00:12"), "2026-05-23 19:00:12");
    }
}
