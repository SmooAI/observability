"""smooai-observability — public types.

Direct port of `packages/core/src/types.ts`. These mirror the Sentry "event
envelope" shape closely enough that the backend can fingerprint and store them
without a parallel schema, while staying first-class for Smoo (no Sentry dep,
no Sentry DSN).

Python uses ``snake_case`` field names internally, but the *wire format* POSTed
to the ingest endpoint must match the TS SDK exactly (``camelCase`` keys,
omit-when-undefined). ``to_wire`` on each dataclass handles that mapping — do
NOT serialize the dataclasses directly with ``dataclasses.asdict``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal

Level = Literal["fatal", "error", "warning", "info", "debug"]
Runtime = Literal["browser", "node", "python"]


def _put(d: dict[str, Any], key: str, value: Any) -> None:
    """Set ``key`` only when ``value`` is not ``None`` — mirrors TS's
    omit-undefined serialization so the wire format matches byte-for-byte
    for present/absent optional fields."""
    if value is not None:
        d[key] = value


@dataclass
class StackFrame:
    """A single stack frame. Mirrors TS ``StackFrame``."""

    module: str
    function: str | None = None
    lineno: int | None = None
    colno: int | None = None
    in_app: bool | None = None

    def to_wire(self) -> dict[str, Any]:
        d: dict[str, Any] = {"module": self.module}
        _put(d, "function", self.function)
        _put(d, "lineno", self.lineno)
        _put(d, "colno", self.colno)
        _put(d, "inApp", self.in_app)
        return d


@dataclass
class ExceptionInfo:
    """One exception in the chain (innermost first). Mirrors TS
    ``ExceptionInfo``."""

    type: str
    value: str
    stacktrace: list[StackFrame] = field(default_factory=list)
    cause: ExceptionInfo | None = None

    def to_wire(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "type": self.type,
            "value": self.value,
            "stacktrace": {"frames": [f.to_wire() for f in self.stacktrace]},
        }
        if self.cause is not None:
            d["cause"] = self.cause.to_wire()
        return d


@dataclass
class Breadcrumb:
    """A breadcrumb in the buffer leading up to an event. Mirrors TS
    ``Breadcrumb``."""

    timestamp: int
    category: str
    level: Level = "info"
    message: str | None = None
    data: dict[str, Any] | None = None

    def to_wire(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "timestamp": self.timestamp,
            "category": self.category,
            "level": self.level,
        }
        _put(d, "message", self.message)
        _put(d, "data", self.data)
        return d


@dataclass
class User:
    """User / org / session context. Mirrors TS ``ObservabilityEvent['user']``."""

    id: str | None = None
    org_id: str | None = None
    session_id: str | None = None

    def to_wire(self) -> dict[str, Any]:
        d: dict[str, Any] = {}
        _put(d, "id", self.id)
        _put(d, "orgId", self.org_id)
        _put(d, "sessionId", self.session_id)
        return d

    def is_empty(self) -> bool:
        return self.id is None and self.org_id is None and self.session_id is None


@dataclass
class RequestInfo:
    """Request context. Mirrors TS ``RequestInfo``."""

    url: str | None = None
    method: str | None = None
    headers: dict[str, str] | None = None
    query_string: str | None = None

    def to_wire(self) -> dict[str, Any]:
        d: dict[str, Any] = {}
        _put(d, "url", self.url)
        _put(d, "method", self.method)
        _put(d, "headers", self.headers)
        _put(d, "queryString", self.query_string)
        return d


@dataclass
class Sdk:
    """SDK self-identification."""

    name: str
    version: str
    runtime: Runtime

    def to_wire(self) -> dict[str, Any]:
        return {"name": self.name, "version": self.version, "runtime": self.runtime}


@dataclass
class ObservabilityEvent:
    """The event envelope. Mirrors TS ``ObservabilityEvent``."""

    event_id: str
    timestamp: int
    level: Level
    sdk: Sdk
    message: str | None = None
    exception: list[ExceptionInfo] | None = None
    breadcrumbs: list[Breadcrumb] | None = None
    user: User | None = None
    request: RequestInfo | None = None
    tags: dict[str, str] | None = None
    contexts: dict[str, dict[str, Any]] | None = None
    release: str | None = None
    environment: str | None = None

    def to_wire(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "eventId": self.event_id,
            "timestamp": self.timestamp,
            "level": self.level,
            "sdk": self.sdk.to_wire(),
        }
        _put(d, "message", self.message)
        if self.exception is not None:
            d["exception"] = [e.to_wire() for e in self.exception]
        if self.breadcrumbs is not None:
            d["breadcrumbs"] = [b.to_wire() for b in self.breadcrumbs]
        if self.user is not None and not self.user.is_empty():
            d["user"] = self.user.to_wire()
        if self.request is not None:
            d["request"] = self.request.to_wire()
        _put(d, "tags", self.tags)
        _put(d, "contexts", self.contexts)
        _put(d, "release", self.release)
        _put(d, "environment", self.environment)
        return d


@dataclass
class IngestPayload:
    """The transport envelope POSTed to the ingest endpoint. Mirrors TS
    ``IngestPayload`` — discriminated union with ``type: 'error'``."""

    events: list[ObservabilityEvent]
    type: Literal["error"] = "error"

    def to_wire(self) -> dict[str, Any]:
        return {"type": self.type, "events": [e.to_wire() for e in self.events]}
