"""Scope / context — ported from ``packages/core/src/scope.ts``.

The TS SDK uses a module-level scope *stack* (safe under Lambda's
single-request-per-invocation model). Python services are commonly long-lived
and concurrent (FastAPI / asyncio), so this port uses ``contextvars`` instead:
each async task / thread sees its own current scope, and ``with_scope`` pushes a
cloned child scope for the duration of a callable. This is the AsyncLocalStorage
equivalent the TS middleware comment flagged as a future need.
"""

from __future__ import annotations

import contextvars
import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from typing import Any, TypeVar

from .types import Breadcrumb, Level, ObservabilityEvent, User

T = TypeVar("T")

_MAX_BREADCRUMBS = 100


def _now_ms() -> int:
    return int(time.time() * 1000)


class Scope:
    """Per-context state merged into events at capture time."""

    def __init__(self) -> None:
        self._user: User | None = None
        self._tags: dict[str, str] = {}
        self._contexts: dict[str, dict[str, Any]] = {}
        self._breadcrumbs: list[Breadcrumb] = []
        self._max_breadcrumbs = _MAX_BREADCRUMBS

    def set_user(self, user: User | None) -> None:
        self._user = user

    def set_tag(self, key: str, value: str) -> None:
        self._tags[key] = value

    def set_context(self, key: str, ctx: dict[str, Any]) -> None:
        self._contexts[key] = ctx

    def add_breadcrumb(
        self,
        category: str,
        message: str | None = None,
        data: dict[str, Any] | None = None,
        level: Level = "info",
        timestamp: int | None = None,
    ) -> None:
        self._breadcrumbs.append(
            Breadcrumb(
                timestamp=timestamp if timestamp is not None else _now_ms(),
                category=category,
                level=level,
                message=message,
                data=data,
            )
        )
        if len(self._breadcrumbs) > self._max_breadcrumbs:
            # Drop oldest — mirrors TS splice(0, len - max).
            self._breadcrumbs = self._breadcrumbs[len(self._breadcrumbs) - self._max_breadcrumbs :]

    def clear_breadcrumbs(self) -> None:
        self._breadcrumbs.clear()

    def apply_to_event(self, event: ObservabilityEvent) -> ObservabilityEvent:
        """Merge this scope's state into an event. Event-level values win over
        scope-level (mirrors the TS spread order ``{...scope, ...event}``)."""
        # User: merge scope under event.
        if self._user is not None or event.user is not None:
            merged = User(
                id=(event.user.id if event.user else None) or (self._user.id if self._user else None),
                org_id=(event.user.org_id if event.user else None) or (self._user.org_id if self._user else None),
                session_id=(event.user.session_id if event.user else None) or (self._user.session_id if self._user else None),
            )
            event.user = merged
        # Tags: scope first, event overrides.
        if self._tags or event.tags:
            event.tags = {**self._tags, **(event.tags or {})}
        # Contexts: scope first, event overrides.
        if self._contexts or event.contexts:
            event.contexts = {**self._contexts, **(event.contexts or {})}
        # Breadcrumbs: scope buffer then any event-supplied ones.
        event.breadcrumbs = [*self._breadcrumbs, *(event.breadcrumbs or [])]
        return event

    def clone(self) -> Scope:
        s = Scope()
        s._user = self._user
        s._tags = dict(self._tags)
        s._contexts = dict(self._contexts)
        s._breadcrumbs = list(self._breadcrumbs)
        return s


_current_scope: contextvars.ContextVar[Scope] = contextvars.ContextVar("smooai_observability_scope")


def get_current_scope() -> Scope:
    """Return the current context's scope, creating a root scope on first use.

    The root scope is created lazily and stored on the var so all callers in a
    context share it (matching the TS single-root-scope default)."""
    try:
        return _current_scope.get()
    except LookupError:
        root = Scope()
        _current_scope.set(root)
        return root


@contextmanager
def with_scope() -> Iterator[Scope]:
    """Push a cloned child scope for the duration of the ``with`` block.

    Async-safe: the clone is bound to a contextvars token so concurrent tasks
    don't see each other's scope mutations. Mirrors TS ``withScope`` — the child
    inherits the parent's user/tags/contexts/breadcrumbs."""
    child = get_current_scope().clone()
    token = _current_scope.set(child)
    try:
        yield child
    finally:
        _current_scope.reset(token)


def run_with_scope[T](fn: Callable[[Scope], T]) -> T:
    """Functional form of ``with_scope`` — mirrors the TS ``withScope(fn)``
    signature for callers that prefer a callback."""
    with with_scope() as scope:
        return fn(scope)
