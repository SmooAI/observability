//! Scope / context — per-task user, tags, contexts, and a bounded breadcrumb
//! buffer.
//!
//! The TS SDK keeps a module-level stack of `Scope`s and pushes/pops around
//! `withScope`. In Rust the natural per-async-task analogue is a
//! [`tokio::task_local`]: each task (or `with_scope` block) sees its own scope,
//! and `current_scope()` reads the innermost. Mutations are interior-mutable
//! (`Mutex`) so `&Scope` callers (set_user / add_breadcrumb) don't need `&mut`.
//!
//! A process-wide [`global_scope`] backs callers that aren't inside a
//! `with_scope` block (e.g. a bare `capture_exception` at startup), mirroring
//! the TS base scope at the bottom of the stack.

use crate::types::{Breadcrumb, Level, ObservabilityEvent, RequestInfo, UserContext};
use once_cell::sync::Lazy;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const MAX_BREADCRUMBS: usize = 100;

#[derive(Default)]
struct ScopeState {
    user: Option<UserContext>,
    tags: BTreeMap<String, String>,
    contexts: BTreeMap<String, serde_json::Value>,
    request: Option<RequestInfo>,
    breadcrumbs: Vec<Breadcrumb>,
}

/// A unit of contextual state merged into events at capture time. Cheap to
/// clone (`Arc`-shared interior); cloning shares the SAME state — use
/// [`Scope::fork`] for an independent copy (what `with_scope` does).
#[derive(Clone)]
pub struct Scope {
    state: Arc<Mutex<ScopeState>>,
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

impl Scope {
    pub fn new() -> Self {
        Scope {
            state: Arc::new(Mutex::new(ScopeState::default())),
        }
    }

    /// Set (or clear) the user context.
    pub fn set_user(&self, user: Option<UserContext>) {
        if let Ok(mut s) = self.state.lock() {
            s.user = user;
        }
    }

    /// Set a single filterable tag.
    pub fn set_tag(&self, key: impl Into<String>, value: impl Into<String>) {
        if let Ok(mut s) = self.state.lock() {
            s.tags.insert(key.into(), value.into());
        }
    }

    /// Set a free-form context block (e.g. `"runtime"`, `"device"`).
    pub fn set_context(&self, key: impl Into<String>, ctx: serde_json::Value) {
        if let Ok(mut s) = self.state.lock() {
            s.contexts.insert(key.into(), ctx);
        }
    }

    /// Attach request / invocation info to this scope.
    pub fn set_request(&self, request: RequestInfo) {
        if let Ok(mut s) = self.state.lock() {
            s.request = Some(request);
        }
    }

    /// Append a breadcrumb, evicting the oldest when the buffer exceeds
    /// [`MAX_BREADCRUMBS`].
    pub fn add_breadcrumb(&self, breadcrumb: Breadcrumb) {
        if let Ok(mut s) = self.state.lock() {
            s.breadcrumbs.push(breadcrumb);
            let len = s.breadcrumbs.len();
            if len > MAX_BREADCRUMBS {
                s.breadcrumbs.drain(0..len - MAX_BREADCRUMBS);
            }
        }
    }

    /// Convenience: add a breadcrumb from parts (matches the TS
    /// `Client.addBreadcrumb` ergonomics).
    pub fn add_breadcrumb_parts(
        &self,
        category: impl Into<String>,
        message: Option<String>,
        data: Option<serde_json::Value>,
        level: Level,
    ) {
        self.add_breadcrumb(Breadcrumb {
            timestamp: crate::now_ms(),
            category: category.into(),
            level,
            message,
            data,
        });
    }

    pub fn clear_breadcrumbs(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.breadcrumbs.clear();
        }
    }

    /// Merge this scope's state into an event. Event-provided fields win over
    /// scope fields, matching the TS `applyToEvent` spread semantics.
    pub fn apply_to_event(&self, mut event: ObservabilityEvent) -> ObservabilityEvent {
        let Ok(s) = self.state.lock() else {
            return event;
        };

        // user: merge scope under event.
        let merged_user = merge_user(s.user.clone(), event.user.take());
        if let Some(u) = merged_user {
            if !u.is_empty() {
                event.user = Some(u);
            }
        }

        // tags: scope first, event overrides.
        if !s.tags.is_empty() || event.tags.is_some() {
            let mut tags = s.tags.clone();
            if let Some(event_tags) = event.tags.take() {
                tags.extend(event_tags);
            }
            if !tags.is_empty() {
                event.tags = Some(tags);
            }
        }

        // contexts: scope first, event overrides.
        if !s.contexts.is_empty() || event.contexts.is_some() {
            let mut ctxs = s.contexts.clone();
            if let Some(event_ctxs) = event.contexts.take() {
                ctxs.extend(event_ctxs);
            }
            if !ctxs.is_empty() {
                event.contexts = Some(ctxs);
            }
        }

        // request: event wins, else scope.
        if event.request.is_none() {
            if let Some(req) = s.request.clone() {
                if !req.is_empty() {
                    event.request = Some(req);
                }
            }
        }

        // breadcrumbs: scope first, then event's own (chronological).
        if !s.breadcrumbs.is_empty() || event.breadcrumbs.is_some() {
            let mut crumbs = s.breadcrumbs.clone();
            if let Some(event_crumbs) = event.breadcrumbs.take() {
                crumbs.extend(event_crumbs);
            }
            if !crumbs.is_empty() {
                event.breadcrumbs = Some(crumbs);
            }
        }

        event
    }

    /// Produce an independent deep copy — used by `with_scope` so child mutations
    /// don't leak into the parent.
    pub fn fork(&self) -> Scope {
        let snapshot = match self.state.lock() {
            Ok(s) => ScopeState {
                user: s.user.clone(),
                tags: s.tags.clone(),
                contexts: s.contexts.clone(),
                request: s.request.clone(),
                breadcrumbs: s.breadcrumbs.clone(),
            },
            Err(_) => ScopeState::default(),
        };
        Scope {
            state: Arc::new(Mutex::new(snapshot)),
        }
    }
}

fn merge_user(base: Option<UserContext>, over: Option<UserContext>) -> Option<UserContext> {
    match (base, over) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (Some(b), Some(o)) => Some(UserContext {
            id: o.id.or(b.id),
            org_id: o.org_id.or(b.org_id),
            session_id: o.session_id.or(b.session_id),
        }),
    }
}

static GLOBAL_SCOPE: Lazy<Scope> = Lazy::new(Scope::new);

tokio::task_local! {
    static CURRENT_SCOPE: Scope;
}

/// The process-wide scope used when no `with_scope` is active. Mutations here
/// apply to every capture that isn't inside a task-local scope.
pub fn global_scope() -> Scope {
    GLOBAL_SCOPE.clone()
}

/// The scope in effect for the current async task: the innermost `with_scope`
/// block if any, else the global scope.
pub fn current_scope() -> Scope {
    CURRENT_SCOPE
        .try_with(|s| s.clone())
        .unwrap_or_else(|_| global_scope())
}

/// Run `fut` with a fresh child scope forked from the current one. The closure
/// receives the child scope so it can set request-specific user/tags. Child
/// mutations do NOT leak into the parent. This is the async-aware analogue of
/// the TS `withScope`.
pub async fn with_scope<F, Fut, T>(f: F) -> T
where
    F: FnOnce(Scope) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let child = current_scope().fork();
    let child_for_closure = child.clone();
    CURRENT_SCOPE.scope(child, f(child_for_closure)).await
}

/// Synchronous variant of [`with_scope`] for non-async call sites.
pub fn with_scope_sync<F, T>(f: F) -> T
where
    F: FnOnce(&Scope) -> T,
{
    let child = current_scope().fork();
    CURRENT_SCOPE.sync_scope(child.clone(), || f(&child))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Level, Runtime, SdkInfo};

    fn blank_event() -> ObservabilityEvent {
        ObservabilityEvent {
            event_id: "id".into(),
            timestamp: 0,
            level: Level::Error,
            message: None,
            exception: None,
            breadcrumbs: None,
            user: None,
            request: None,
            tags: None,
            contexts: None,
            release: None,
            environment: None,
            sdk: SdkInfo {
                name: "@smooai/observability".into(),
                version: "0.1.0".into(),
                runtime: Runtime::Node,
            },
        }
    }

    #[test]
    fn apply_merges_tags_and_user() {
        let scope = Scope::new();
        scope.set_tag("a", "1");
        scope.set_user(Some(UserContext {
            id: Some("u1".into()),
            org_id: Some("o1".into()),
            session_id: None,
        }));
        let event = scope.apply_to_event(blank_event());
        assert_eq!(event.tags.unwrap()["a"], "1");
        let user = event.user.unwrap();
        assert_eq!(user.id.unwrap(), "u1");
        assert_eq!(user.org_id.unwrap(), "o1");
    }

    #[test]
    fn breadcrumbs_capped_at_100() {
        let scope = Scope::new();
        for i in 0..150 {
            scope.add_breadcrumb_parts("test", Some(format!("crumb {i}")), None, Level::Info);
        }
        let event = scope.apply_to_event(blank_event());
        let crumbs = event.breadcrumbs.unwrap();
        assert_eq!(crumbs.len(), 100);
        // Oldest evicted: first remaining should be crumb 50.
        assert_eq!(crumbs[0].message.as_deref(), Some("crumb 50"));
        assert_eq!(crumbs[99].message.as_deref(), Some("crumb 149"));
    }

    #[test]
    fn event_fields_win_over_scope() {
        let scope = Scope::new();
        scope.set_tag("env", "scope");
        let mut event = blank_event();
        let mut t = BTreeMap::new();
        t.insert("env".to_string(), "event".to_string());
        event.tags = Some(t);
        let merged = scope.apply_to_event(event);
        assert_eq!(merged.tags.unwrap()["env"], "event");
    }

    #[test]
    fn fork_is_independent() {
        let parent = Scope::new();
        parent.set_tag("a", "1");
        let child = parent.fork();
        child.set_tag("b", "2");
        let parent_event = parent.apply_to_event(blank_event());
        let parent_tags = parent_event.tags.unwrap();
        assert!(parent_tags.contains_key("a"));
        assert!(
            !parent_tags.contains_key("b"),
            "child mutation leaked to parent"
        );
    }

    #[tokio::test]
    async fn with_scope_isolates_context() {
        global_scope().set_tag("global", "yes");
        with_scope(|s| async move {
            s.set_tag("scoped", "yes");
            let event = current_scope().apply_to_event(blank_event());
            let tags = event.tags.unwrap();
            assert_eq!(tags["global"], "yes", "inherits global");
            assert_eq!(tags["scoped"], "yes", "sees own");
        })
        .await;
        // After the block, the global scope must NOT have the scoped tag.
        let after = global_scope().apply_to_event(blank_event());
        assert!(!after.tags.unwrap().contains_key("scoped"));
    }
}
