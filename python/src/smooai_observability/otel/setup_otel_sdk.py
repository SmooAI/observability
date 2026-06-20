"""OpenTelemetry SDK bootstrap — port of
``packages/core/src/otel/setup-otel-sdk.ts``.

Initializes a global TracerProvider + MeterProvider with OTLP/HTTP export. A
single call at process start (server entry, Lambda handler module, or
``bootstrap``). Idempotent — a second call is a no-op so tests and lazy boots
don't double-register exporters. Returns a handle exposing ``flush`` /
``shutdown`` for the host's lifecycle (SIGTERM / atexit).

When a ``TokenProvider`` is supplied, traces + metrics export via the
auth-injecting exporters (fresh Bearer per request). Otherwise the standard
OTLP exporters are used with static ``otlp_headers`` — matching the TS legacy
path for callers that pre-mint their own token.

``opentelemetry-exporter-otlp-proto-http`` is an OPTIONAL dependency (the
``otlp`` extra). If it isn't installed, ``setup_otel_sdk`` logs to stderr and
returns a disabled handle instead of raising — observability never crashes the
host.
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass, field
from typing import Any

from ..auth.token_provider import TokenProvider

DEFAULT_SERVICE_NAME = "smoo-service"
DEFAULT_METRIC_EXPORT_INTERVAL_MS = 30_000


@dataclass
class SetupOtelOptions:
    service_name: str = DEFAULT_SERVICE_NAME
    otlp_endpoint: str | None = None
    otlp_metrics_endpoint: str | None = None
    otlp_headers: dict[str, str] = field(default_factory=dict)
    environment: str | None = None
    release: str | None = None
    token_provider: TokenProvider | None = None
    metric_export_interval_ms: int = DEFAULT_METRIC_EXPORT_INTERVAL_MS
    # Skip starting providers (test seam — construct but don't install globally).
    skip_start: bool = False


@dataclass
class OtelSdkHandle:
    tracer_provider: Any = None
    meter_provider: Any = None
    enabled: bool = False

    def flush(self, timeout_millis: int = 2_000) -> None:
        for provider in (self.tracer_provider, self.meter_provider):
            if provider is None:
                continue
            try:
                provider.force_flush(timeout_millis=timeout_millis)
            except Exception:
                pass

    def shutdown(self) -> None:
        global _installed
        for provider in (self.tracer_provider, self.meter_provider):
            if provider is None:
                continue
            try:
                provider.shutdown()
            except Exception:
                pass
        _installed = None


_installed: OtelSdkHandle | None = None


def _warn(message: str) -> None:
    try:
        sys.stderr.write(f"[@smooai/observability/otel] {message}\n")
    except Exception:
        pass


def setup_otel_sdk(options: SetupOtelOptions | None = None, **kwargs: Any) -> OtelSdkHandle:
    """Set up global trace + metric providers with OTLP/HTTP export."""
    global _installed
    if _installed is not None:
        return _installed

    opts = options or SetupOtelOptions(**kwargs)

    try:
        from opentelemetry import metrics as otel_metrics
        from opentelemetry import trace as otel_trace
        from opentelemetry.sdk.metrics import MeterProvider
        from opentelemetry.sdk.metrics.export import PeriodicExportingMetricReader
        from opentelemetry.sdk.resources import Resource
        from opentelemetry.sdk.trace import TracerProvider
        from opentelemetry.sdk.trace.export import BatchSpanProcessor
    except ImportError as err:  # pragma: no cover - import guard
        _warn(f"opentelemetry-sdk not available; OTel disabled: {err}")
        _installed = OtelSdkHandle(enabled=False)
        return _installed

    try:
        from opentelemetry.exporter.otlp.proto.http.metric_exporter import OTLPMetricExporter
        from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter

        from .auth_injecting_exporter import (
            AuthInjectingMetricExporter,
            AuthInjectingTraceExporter,
        )
    except ImportError as err:
        _warn(f"opentelemetry-exporter-otlp-proto-http not installed (install the 'otlp' extra); OTel export disabled: {err}")
        _installed = OtelSdkHandle(enabled=False)
        return _installed

    try:
        trace_endpoint = opts.otlp_endpoint or os.environ.get("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") or os.environ.get("OTEL_EXPORTER_OTLP_ENDPOINT")
        metric_endpoint = opts.otlp_metrics_endpoint or os.environ.get("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT") or os.environ.get("OTEL_EXPORTER_OTLP_ENDPOINT")

        resource_attrs: dict[str, Any] = {"service.name": opts.service_name}
        if opts.release:
            resource_attrs["service.version"] = opts.release
        if opts.environment:
            resource_attrs["deployment.environment.name"] = opts.environment
        resource = Resource.create(resource_attrs)

        # --- traces ---------------------------------------------------------
        if opts.token_provider is not None and trace_endpoint:
            trace_exporter: Any = AuthInjectingTraceExporter(
                endpoint=trace_endpoint,
                token_provider=opts.token_provider,
                headers=opts.otlp_headers or None,
            )
        elif trace_endpoint:
            trace_exporter = OTLPSpanExporter(endpoint=trace_endpoint, headers=opts.otlp_headers or None)
        else:
            trace_exporter = OTLPSpanExporter(headers=opts.otlp_headers or None)

        tracer_provider = TracerProvider(resource=resource)
        tracer_provider.add_span_processor(BatchSpanProcessor(trace_exporter))

        # --- metrics --------------------------------------------------------
        if opts.token_provider is not None and metric_endpoint:
            metric_exporter: Any = AuthInjectingMetricExporter(
                endpoint=metric_endpoint,
                token_provider=opts.token_provider,
                headers=opts.otlp_headers or None,
            )
        elif metric_endpoint:
            metric_exporter = OTLPMetricExporter(endpoint=metric_endpoint, headers=opts.otlp_headers or None)
        else:
            metric_exporter = OTLPMetricExporter(headers=opts.otlp_headers or None)

        metric_reader = PeriodicExportingMetricReader(
            metric_exporter,
            export_interval_millis=opts.metric_export_interval_ms,
        )
        meter_provider = MeterProvider(resource=resource, metric_readers=[metric_reader])

        if not opts.skip_start:
            otel_trace.set_tracer_provider(tracer_provider)
            otel_metrics.set_meter_provider(meter_provider)

        handle = OtelSdkHandle(
            tracer_provider=tracer_provider,
            meter_provider=meter_provider,
            enabled=True,
        )
        _installed = handle
        return handle
    except Exception as err:
        _warn(f"setup failed; OTel disabled: {err}")
        _installed = OtelSdkHandle(enabled=False)
        return _installed


def reset_otel_sdk_for_tests() -> None:
    """Test seam — wipes the install guard so the next call re-initializes."""
    global _installed
    _installed = None
