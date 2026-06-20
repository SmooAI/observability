"""OTel-native capture handler — port of
``packages/core/src/node/otel-capture.ts``.

Registered as the Client's CaptureHandler. Every captured exception / message
becomes a span event on the active OTel span (or a synthetic one if none is
active), with ``SpanStatusCode.ERROR`` for exceptions and OTLP-shaped attributes
for tags / user / release. The OTel SDK handles batching, retry, wire format.

This runs ALONGSIDE the webhook transport (SMOODEV-1148 dual-path): OTel gets
the structured span event, the webhook gets the event for the Errors dashboard.
"""

from __future__ import annotations

from typing import Any

from opentelemetry import trace
from opentelemetry.trace import SpanKind, Status, StatusCode

from ..client import Client
from ..types import ObservabilityEvent

_installed = False


def register_otel_capture(tracer_name: str = "smooai.observability") -> None:
    """Wire the OTel-native capture handler into the Client (idempotent)."""
    global _installed
    if _installed:
        return
    _installed = True

    tracer = trace.get_tracer(tracer_name)

    def handler(event: ObservabilityEvent, raw: dict[str, Any]) -> None:
        active = trace.get_current_span()
        # An invalid/non-recording span means there's no real active span.
        if active is not None and active.get_span_context().is_valid:
            _record_on_span(active, event, raw)
            return
        name = "observability.captureException" if event.exception else "observability.captureMessage"
        span = tracer.start_span(name, kind=SpanKind.INTERNAL)
        try:
            _record_on_span(span, event, raw)
        finally:
            span.end()

    Client.register_capture_handler(handler)


def reset_otel_capture_for_tests() -> None:
    global _installed
    _installed = False
    Client.register_capture_handler(None)


def _record_on_span(span: Any, event: ObservabilityEvent, raw: dict[str, Any]) -> None:
    is_exception = bool(event.exception)
    attrs: dict[str, Any] = {"smoo.event_id": event.event_id}
    if event.environment:
        attrs["deployment.environment.name"] = event.environment
    if event.release:
        attrs["service.version"] = event.release
    if event.level:
        attrs["smoo.level"] = event.level
    if event.user:
        if event.user.id:
            attrs["enduser.id"] = event.user.id
        if event.user.org_id:
            attrs["enduser.org_id"] = event.user.org_id
        if event.user.session_id:
            attrs["enduser.session_id"] = event.user.session_id
    if event.tags:
        for k, v in event.tags.items():
            attrs[f"smoo.tag.{k}"] = v

    if is_exception:
        err = raw.get("error")
        if isinstance(err, BaseException):
            span.record_exception(err)
        else:
            span.record_exception(Exception(err if isinstance(err, str) else "non-Error captured"))
        msg = str(err) if isinstance(err, BaseException) else (event.exception[0].value if event.exception else None)
        span.set_status(Status(StatusCode.ERROR, msg))
    elif event.message:
        span.add_event("smoo.message", {**attrs, "smoo.message": event.message})
        if event.level in ("error", "fatal"):
            span.set_status(Status(StatusCode.ERROR, event.message))

    if attrs:
        span.set_attributes(attrs)
