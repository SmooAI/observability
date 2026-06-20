"""Shared test config.

Silences the OTLP exporter's "connection refused to localhost:4318" log spam:
some tests construct real OTLP exporters (to assert wiring) that try to flush on
shutdown against a collector that isn't running in CI. That's expected — raise
the exporter loggers above ERROR so the tracebacks don't drown the report.
"""

import logging

for name in (
    "opentelemetry.exporter.otlp.proto.http.trace_exporter",
    "opentelemetry.exporter.otlp.proto.http.metric_exporter",
    "opentelemetry.sdk.trace.export",
    "opentelemetry.sdk.metrics.export",
):
    logging.getLogger(name).setLevel(logging.CRITICAL)
