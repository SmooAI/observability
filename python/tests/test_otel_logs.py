"""OTLP logs signal + trace correlation tests.

The logs signal reuses the same endpoint/auth/enable path as traces + metrics
and bridges stdlib ``logging`` → OTel log records via ``LoggingHandler``. The
load-bearing behavior is that a log emitted inside an active span carries that
span's real W3C trace_id/span_id — that's what correlates logs to traces in the
product. We verify it against the exact ``LoggingHandler`` class ``setup_otel_sdk``
attaches, using an in-memory exporter (no network).
"""

from __future__ import annotations

import logging

import pytest
from opentelemetry.sdk._logs import LoggerProvider, LoggingHandler
from opentelemetry.sdk._logs.export import InMemoryLogExporter, SimpleLogRecordProcessor
from opentelemetry.sdk.trace import TracerProvider

from smooai_observability.otel import (
    SetupOtelOptions,
    reset_otel_sdk_for_tests,
    setup_otel_sdk,
)


@pytest.fixture(autouse=True)
def _reset():
    reset_otel_sdk_for_tests()
    yield
    reset_otel_sdk_for_tests()


def test_setup_wires_logger_provider():
    handle = setup_otel_sdk(
        SetupOtelOptions(
            service_name="svc",
            otlp_logs_endpoint="https://example.test/v1/logs",
            skip_start=True,
        )
    )
    assert handle.enabled is True
    assert handle.logger_provider is not None
    assert handle.log_handler is not None


def test_skip_start_does_not_touch_root_logger():
    before = list(logging.getLogger().handlers)
    setup_otel_sdk(
        SetupOtelOptions(
            service_name="svc",
            otlp_logs_endpoint="https://example.test/v1/logs",
            skip_start=True,
        )
    )
    assert list(logging.getLogger().handlers) == before


def test_log_within_span_carries_trace_context():
    exporter = InMemoryLogExporter()
    provider = LoggerProvider()
    provider.add_log_record_processor(SimpleLogRecordProcessor(exporter))
    handler = LoggingHandler(level=logging.NOTSET, logger_provider=provider)

    log = logging.getLogger("bridge_correlation_test")
    log.setLevel(logging.DEBUG)
    log.propagate = False
    log.addHandler(handler)

    tracer = TracerProvider().get_tracer("test")
    with tracer.start_as_current_span("work") as span:
        ctx = span.get_span_context()
        log.warning("inside the span")

    records = exporter.get_finished_logs()
    assert records, "expected a log record to be exported"
    lr = records[-1].log_record
    # Real W3C ids from the active span — not a fabricated/zero id.
    assert lr.trace_id == ctx.trace_id
    assert lr.span_id == ctx.span_id
    assert lr.trace_id != 0 and lr.span_id != 0


def test_log_outside_span_has_no_trace_context():
    exporter = InMemoryLogExporter()
    provider = LoggerProvider()
    provider.add_log_record_processor(SimpleLogRecordProcessor(exporter))
    handler = LoggingHandler(level=logging.NOTSET, logger_provider=provider)

    log = logging.getLogger("bridge_no_span_test")
    log.setLevel(logging.DEBUG)
    log.propagate = False
    log.addHandler(handler)

    log.warning("no active span")

    records = exporter.get_finished_logs()
    assert records
    # No span active → zero ids (OTel's "invalid" sentinel), not a random uuid.
    assert records[-1].log_record.trace_id == 0
