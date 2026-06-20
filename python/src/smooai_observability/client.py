"""Singleton client — ported from ``packages/core/src/client.ts``.

``capture_exception`` / ``capture_message`` prepare an ``ObservabilityEvent``
(scope-merged), run it through ``before_send``, then fire BOTH registered paths
(SMOODEV-1148 parity):

  * the **capture handler** (e.g. OTel-native span events), and
  * the **webhook transport** (POSTs to the DSN for the Errors dashboard).

Order: handler first, so a throwing transport can't suppress OTel capture. Every
public method is error-safe — observability must never throw into user code.
"""

from __future__ import annotations

import time
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Protocol

from . import pii
from .scope import get_current_scope
from .stack_parser import drop_sdk_frames, parse_traceback
from .types import (
    ExceptionInfo,
    Level,
    ObservabilityEvent,
    Runtime,
    Sdk,
    User,
)

SDK_NAME = "@smooai/observability"
SDK_VERSION = "0.1.0"
RUNTIME: Runtime = "python"


@dataclass
class ClientOptions:
    """Mirrors TS ``ClientOptions`` (Python-relevant subset)."""

    dsn: str | None = None
    environment: str | None = None
    release: str | None = None
    max_queue_size: int | None = None
    flush_interval_ms: int | None = None
    max_batch_size: int | None = None
    # Drop / mutate events before send. Return None to drop.
    before_send: Callable[[ObservabilityEvent], ObservabilityEvent | None] | None = None
    # Scrub PII from messages / breadcrumbs before send. Default True.
    scrub_pii: bool = True


class CaptureHandler(Protocol):
    """Runtime-native capture path (e.g. OTel span events). Mirrors TS
    ``CaptureHandler``."""

    def __call__(self, event: ObservabilityEvent, raw: dict[str, Any]) -> None: ...


Transport = Callable[[list[ObservabilityEvent]], None]


class _Client:
    def __init__(self) -> None:
        self._options: ClientOptions | None = None
        self._transport: Transport | None = None
        self._capture_handler: CaptureHandler | None = None

    def init(self, options: ClientOptions) -> None:
        self._options = options

    def is_initialized(self) -> bool:
        return self._options is not None

    def get_options(self) -> ClientOptions | None:
        return self._options

    def register_transport(self, transport: Transport | None) -> None:
        self._transport = transport

    def register_capture_handler(self, handler: CaptureHandler | None) -> None:
        self._capture_handler = handler

    # --- scope convenience (mirror TS setUser/setTag/addBreadcrumb) ----------

    def set_user(self, user: User | None) -> None:
        get_current_scope().set_user(user)

    def set_tag(self, key: str, value: str) -> None:
        get_current_scope().set_tag(key, value)

    def set_context(self, key: str, ctx: dict[str, Any]) -> None:
        get_current_scope().set_context(key, ctx)

    def add_breadcrumb(
        self,
        category: str,
        message: str | None = None,
        data: dict[str, Any] | None = None,
        level: Level = "info",
    ) -> None:
        get_current_scope().add_breadcrumb(category, message, data, level)

    # --- capture -------------------------------------------------------------

    def capture_exception(
        self,
        error: BaseException,
        tags: dict[str, str] | None = None,
    ) -> str | None:
        if self._options is None:
            return None
        try:
            event_id = str(uuid.uuid4())
            exc = _to_exception(error)
            event = get_current_scope().apply_to_event(
                ObservabilityEvent(
                    event_id=event_id,
                    timestamp=_now_ms(),
                    level="error",
                    exception=[exc],
                    tags=tags,
                    release=self._options.release,
                    environment=self._options.environment,
                    sdk=Sdk(SDK_NAME, SDK_VERSION, RUNTIME),
                )
            )
            return self._dispatch(event, {"error": error, "extra": {"tags": tags}})
        except Exception:
            return None

    def capture_message(
        self,
        message: str,
        level: Level = "info",
    ) -> str | None:
        if self._options is None:
            return None
        try:
            event_id = str(uuid.uuid4())
            scrubbed = pii.scrub_string(message) if self._options.scrub_pii else message
            event = get_current_scope().apply_to_event(
                ObservabilityEvent(
                    event_id=event_id,
                    timestamp=_now_ms(),
                    level=level,
                    message=scrubbed,
                    release=self._options.release,
                    environment=self._options.environment,
                    sdk=Sdk(SDK_NAME, SDK_VERSION, RUNTIME),
                )
            )
            return self._dispatch(event, {"message": message})
        except Exception:
            return None

    def _dispatch(self, event: ObservabilityEvent, raw: dict[str, Any]) -> str | None:
        assert self._options is not None
        if self._options.scrub_pii:
            _scrub_event(event)
        final = event
        if self._options.before_send is not None:
            try:
                result = self._options.before_send(event)
            except Exception:
                result = event  # a throwing before_send must not drop the event
            if result is None:
                return event.event_id
            final = result
        # SMOODEV-1148: fire BOTH paths. Handler first so a throwing transport
        # doesn't suppress OTel capture.
        if self._capture_handler is not None:
            try:
                self._capture_handler(final, raw)
            except Exception:
                pass  # observability must not throw
        if self._transport is not None:
            try:
                self._transport([final])
            except Exception:
                pass
        return final.event_id


def _now_ms() -> int:
    return int(time.time() * 1000)


def _to_exception(err: BaseException) -> ExceptionInfo:
    """Build the (possibly chained) ExceptionInfo from a Python exception.

    Walks ``__cause__`` (explicit ``raise from``) then ``__context__``
    (implicit) — the Python analog of JS ``Error.cause`` chaining."""
    exc_type = type(err).__name__
    value = str(err)
    frames = drop_sdk_frames(parse_traceback(err.__traceback__))
    info = ExceptionInfo(type=exc_type, value=value, stacktrace=frames)

    cause = err.__cause__ if err.__cause__ is not None else err.__context__
    if cause is not None and cause is not err:
        try:
            info.cause = _to_exception(cause)
        except RecursionError:
            pass
    return info


def _scrub_event(event: ObservabilityEvent) -> None:
    """Scrub PII in-place on message, breadcrumb messages, and request headers."""
    if event.message:
        event.message = pii.scrub_string(event.message)
    if event.breadcrumbs:
        for bc in event.breadcrumbs:
            if bc.message:
                bc.message = pii.scrub_string(bc.message)
    if event.exception:
        for exc in event.exception:
            if exc.value:
                exc.value = pii.scrub_string(exc.value)
    if event.request and event.request.headers:
        event.request.headers = pii.scrub_headers(event.request.headers)


# Module-level singleton — mirrors TS `export const Client`.
Client = _Client()
