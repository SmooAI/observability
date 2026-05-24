//! Settings dialog — list connected orgs + add/verify/remove. Lives in a
//! single component file because the form state is tightly coupled to the
//! list state and they share the registry signal.

use std::sync::Arc;

use dioxus::prelude::*;
use observability_studio_client::auth;
use uuid::Uuid;

use crate::components::icons::*;
use crate::persistence::{OrgEntry, OrgRegistry};
use crate::state::ApiState;

#[component]
pub fn SettingsDialog() -> Element {
    let mut open = use_context::<Signal<bool>>();
    let mut registry = use_context::<Signal<OrgRegistry>>();

    let close = move |_| open.set(false);

    let orgs = registry().entries.clone();

    rsx! {
        div { class: "dialog__backdrop", onclick: close,
            // Stop click-through so clicks inside the dialog don't dismiss it.
            div { class: "dialog", onclick: move |evt| evt.stop_propagation(),
                div { class: "dialog__header",
                    div {
                        div { class: "dialog__title", "Settings" }
                        div { class: "dialog__desc",
                            "Connect SmooAI orgs by M2M client credentials. Secrets are stored in the OS keychain — never on disk."
                        }
                    }
                    button {
                        class: "dialog__close",
                        onclick: close,
                        XIcon {}
                    }
                }

                section { class: "dialog__section",
                    div { class: "dialog__section-title", "Connected orgs" }
                    if orgs.is_empty() {
                        div { class: "empty", "No orgs added yet." }
                    } else {
                        for entry in orgs.iter().cloned() {
                            {
                                let key = entry.org_id.to_string();
                                rsx! {
                                    OrgRow {
                                        key: "{key}",
                                        entry: entry.clone(),
                                        on_removed: move |id: Uuid| {
                                            let mut r = registry();
                                            r.remove(id);
                                            r.save();
                                            registry.set(r);
                                        },
                                    }
                                }
                            }
                        }
                    }
                }

                div { class: "dialog__divider" }

                AddOrgForm {
                    on_added: move |entry: OrgEntry| {
                        let mut r = registry();
                        r.upsert(entry);
                        r.save();
                        registry.set(r);
                    },
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct OrgRowProps {
    entry: OrgEntry,
    on_removed: EventHandler<Uuid>,
}

#[component]
fn OrgRow(props: OrgRowProps) -> Element {
    let mut removing = use_signal(|| false);
    let entry = props.entry.clone();
    let id_full = entry.org_id.to_string();
    let id_short = format!("{}…{}", &id_full[..8], &id_full[id_full.len() - 4..]);
    let cid_short = format!("{}…", entry.client_id_preview.chars().take(8).collect::<String>());

    // Pull the ApiState from context inside the component so the Props can stay
    // `PartialEq`-clean (Arc<ApiState> isn't PartialEq).
    let api_state = use_context::<Arc<ApiState>>();
    let on_removed = props.on_removed;
    let click_id = entry.org_id;
    let remove = move |_| {
        let api_state = api_state.clone();
        spawn(async move {
            removing.set(true);
            // Best-effort keychain wipe + cache invalidate; surface nothing if
            // the keychain entry was already missing.
            let _ = auth::remove_credentials(click_id);
            api_state.auth.invalidate(click_id);
            on_removed.call(click_id);
            removing.set(false);
        });
    };

    rsx! {
        div { class: "org-row",
            div { class: "org-row__main",
                span { class: "org-row__label", "{entry.label}" }
                span { class: "org-row__meta", "{id_short} · {entry.base_url}" }
            }
            span { class: "org-row__cid", "{cid_short}" }
            button {
                class: "btn btn--ghost btn--icon",
                title: "Remove",
                disabled: removing(),
                onclick: remove,
                if removing() { div { class: "spinner" } } else { TrashIcon {} }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct AddOrgFormProps {
    on_added: EventHandler<OrgEntry>,
}

#[component]
fn AddOrgForm(props: AddOrgFormProps) -> Element {
    let mut label = use_signal(String::new);
    let mut org_id = use_signal(String::new);
    let base_url = use_signal(|| "https://api.smoo.ai".to_string());
    let mut client_id = use_signal(String::new);
    let mut client_secret = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error_msg = use_signal::<Option<String>>(|| None);

    let api_state = use_context::<Arc<ApiState>>();
    let on_added = props.on_added;

    let submit = move |_| {
        if busy() {
            return;
        }
        let label_v = label().trim().to_string();
        let org_id_v = org_id().trim().to_string();
        let base_url_v = {
            let t = base_url().trim().to_string();
            if t.is_empty() { "https://api.smoo.ai".to_string() } else { t }
        };
        let client_id_v = client_id().trim().to_string();
        let client_secret_v = client_secret().trim().to_string();

        if label_v.is_empty()
            || org_id_v.is_empty()
            || client_id_v.is_empty()
            || client_secret_v.is_empty()
        {
            error_msg.set(Some("All fields are required.".into()));
            return;
        }

        let org_uuid = match Uuid::parse_str(&org_id_v) {
            Ok(u) => u,
            Err(e) => {
                error_msg.set(Some(format!("Org ID is not a valid UUID: {e}")));
                return;
            }
        };

        error_msg.set(None);
        busy.set(true);
        let api_state = api_state.clone();
        let label_v_c = label_v.clone();
        let base_url_v_c = base_url_v.clone();
        let client_id_v_c = client_id_v.clone();

        spawn(async move {
            // Verify the candidate creds against /token before touching the
            // keychain — a typo here shouldn't leave bad creds behind.
            let verify =
                api_state.auth.verify(&client_id_v, &client_secret_v).await;
            match verify {
                Err(e) => {
                    error_msg.set(Some(render_auth_error(e)));
                    busy.set(false);
                    return;
                }
                Ok(_) => {}
            }
            if let Err(e) = auth::store_credentials(
                org_uuid,
                &client_id_v,
                &client_secret_v,
            ) {
                error_msg.set(Some(format!("Keychain write failed: {e}")));
                busy.set(false);
                return;
            }
            on_added.call(OrgEntry {
                org_id: org_uuid,
                label: label_v_c,
                base_url: base_url_v_c,
                client_id_preview: client_id_v_c,
            });
            label.set(String::new());
            org_id.set(String::new());
            client_id.set(String::new());
            client_secret.set(String::new());
            busy.set(false);
        });
    };

    rsx! {
        div { class: "dialog__section-title", "Add an org" }
        div { class: "form",
            div { class: "form__grid",
                Field {
                    label: "Label".to_string(),
                    placeholder: "smoo prod".to_string(),
                    value: label,
                    mono: false,
                    full: false,
                    secret: false,
                }
                Field {
                    label: "Org ID".to_string(),
                    placeholder: "UUID".to_string(),
                    value: org_id,
                    mono: true,
                    full: false,
                    secret: false,
                }
                Field {
                    label: "Base URL".to_string(),
                    placeholder: "https://api.smoo.ai".to_string(),
                    value: base_url,
                    mono: true,
                    full: true,
                    secret: false,
                }
                Field {
                    label: "client_id".to_string(),
                    placeholder: "UUID".to_string(),
                    value: client_id,
                    mono: true,
                    full: true,
                    secret: false,
                }
                Field {
                    label: "client_secret".to_string(),
                    placeholder: "sk_…".to_string(),
                    value: client_secret,
                    mono: true,
                    full: true,
                    secret: true,
                }
            }
            if let Some(msg) = error_msg() {
                div { class: "form__error",
                    span { style: "width:14px;height:14px;display:inline-flex;", XIcon {} }
                    "{msg}"
                }
            }
            div { class: "form__actions",
                button {
                    class: "btn btn--primary",
                    disabled: busy(),
                    onclick: submit,
                    if busy() {
                        div { class: "spinner" }
                    } else {
                        span { style: "width:14px;height:14px;display:inline-flex;", PlusIcon {} }
                    }
                    "Verify & save"
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct FieldProps {
    label: String,
    placeholder: String,
    value: Signal<String>,
    mono: bool,
    full: bool,
    secret: bool,
}

#[component]
fn Field(props: FieldProps) -> Element {
    let class = if props.full { "field field--full" } else { "field" };
    let input_class = if props.mono {
        "field__input field__input--mono"
    } else {
        "field__input"
    };
    let input_type = if props.secret { "password" } else { "text" };
    let mut value = props.value;
    rsx! {
        div { class: "{class}",
            label { class: "field__label", "{props.label}" }
            input {
                class: "{input_class}",
                r#type: "{input_type}",
                placeholder: "{props.placeholder}",
                value: "{value()}",
                oninput: move |evt| value.set(evt.value()),
            }
        }
    }
}

fn render_auth_error(e: observability_studio_client::AuthError) -> String {
    use observability_studio_client::AuthError;
    match e {
        AuthError::TokenEndpoint(status) => {
            format!("auth.smoo.ai rejected the credentials ({status})")
        }
        other => other.to_string(),
    }
}
