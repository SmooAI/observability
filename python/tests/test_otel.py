"""OTel setup + native-capture tests.

OTel's global ``set_tracer_provider`` may only be set ONCE per process. We
install a single module-scoped TracerProvider + InMemorySpanExporter, and clear
the exporter between tests rather than swapping providers.
"""

from __future__ import annotations

import pytest
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import (
    InMemorySpanExporter,
)

from smooai_observability.client import Client, ClientOptions
from smooai_observability.otel import (
    SetupOtelOptions,
    register_otel_capture,
    reset_otel_capture_for_tests,
    reset_otel_sdk_for_tests,
    setup_otel_sdk,
)

_EXPORTER = InMemorySpanExporter()
_PROVIDER = TracerProvider()
_PROVIDER.add_span_processor(SimpleSpanProcessor(_EXPORTER))
trace.set_tracer_provider(_PROVIDER)


@pytest.fixture(autouse=True)
def _reset():
    reset_otel_sdk_for_tests()
    reset_otel_capture_for_tests()
    _EXPORTER.clear()
    yield
    reset_otel_sdk_for_tests()
    reset_otel_capture_for_tests()
    Client._options = None


def test_setup_is_idempotent():
    h1 = setup_otel_sdk(SetupOtelOptions(service_name="svc", skip_start=True))
    h2 = setup_otel_sdk(SetupOtelOptions(service_name="svc2", skip_start=True))
    assert h1 is h2  # second call returns the same handle


def test_setup_returns_enabled_handle():
    handle = setup_otel_sdk(
        SetupOtelOptions(
            service_name="svc",
            otlp_endpoint="https://example.test/v1/traces",
            skip_start=True,
        )
    )
    assert handle.enabled is True
    assert handle.tracer_provider is not None
    assert handle.meter_provider is not None


def test_otel_capture_records_exception_on_active_span():
    Client.init(ClientOptions(environment="test", release="r1"))
    Client.register_transport(None)
    register_otel_capture()
    tracer = trace.get_tracer("test")
    # Capture while the span is still active (the production pattern — the
    # middleware catches the error before the request span ends). Attributes
    # written after a span ends are silently dropped by OTel.
    with tracer.start_as_current_span("work"):
        try:
            raise ValueError("boom in span")
        except ValueError as err:
            Client.capture_exception(err, tags={"k": "v"})
    spans = _EXPORTER.get_finished_spans()
    work = next(s for s in spans if s.name == "work")
    assert work.status.status_code.name == "ERROR"
    assert work.attributes["smoo.tag.k"] == "v"
    assert any(e.name == "exception" for e in work.events)


def test_otel_capture_synthetic_span_when_none_active():
    Client.init(ClientOptions(environment="test"))
    Client.register_transport(None)
    register_otel_capture()
    Client.capture_message("standalone message", "error")
    spans = _EXPORTER.get_finished_spans()
    synthetic = next(s for s in spans if s.name == "observability.captureMessage")
    assert any(e.name == "smoo.message" for e in synthetic.events)
