"""smooai_observability.metrics — OpenTelemetry Meter wrapper.

Port of ``packages/core/src/metrics/index.ts``. A tiny ergonomic layer over the
OTel metrics API so consumers don't have to learn it just to emit counters.

    from smooai_observability.otel import setup_otel_sdk
    from smooai_observability.metrics import get_metrics_client

    setup_otel_sdk(service_name="smooai-voice")  # also wires metrics export
    metrics = get_metrics_client("smooai-voice")

    metrics.counter("agent.turn.completed", 1, {"channel": "voice", "tier": "pro"})
    metrics.timing("agent.ttft.ms", 312, {"model": "sonnet"})
    stop = metrics.start_timer("agent.tool.latency.ms", {"tool": "knowledge_search"})
    do_work()
    stop()

Instruments are cached by ``(meter_name, instrument_name[, unit])`` so we don't
leak Meter handles. Every method swallows exceptions — metrics must never throw
into user code.
"""

from __future__ import annotations

import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from typing import Any

from opentelemetry import metrics as otel_metrics

DEFAULT_METER_NAME = "@smooai/observability"

_counter_cache: dict[str, Any] = {}
_histogram_cache: dict[str, Any] = {}


def _get_counter(meter_name: str, name: str) -> Any:
    key = f"{meter_name}::{name}"
    inst = _counter_cache.get(key)
    if inst is None:
        inst = otel_metrics.get_meter(meter_name).create_counter(name)
        _counter_cache[key] = inst
    return inst


def _get_histogram(meter_name: str, name: str, unit: str | None = None) -> Any:
    key = f"{meter_name}::{name}::{unit or ''}"
    inst = _histogram_cache.get(key)
    if inst is None:
        inst = otel_metrics.get_meter(meter_name).create_histogram(name, unit=unit or "")
        _histogram_cache[key] = inst
    return inst


class MetricsClient:
    """Service-bound metrics client. Mirrors the TS ``MetricsClient`` shape."""

    def __init__(self, meter_name: str) -> None:
        self._meter_name = meter_name

    def counter(self, name: str, value: float = 1, attrs: dict[str, str] | None = None) -> None:
        try:
            _get_counter(self._meter_name, name).add(value, attrs or {})
        except Exception:
            pass

    def histogram(self, name: str, value: float, attrs: dict[str, str] | None = None) -> None:
        try:
            _get_histogram(self._meter_name, name).record(value, attrs or {})
        except Exception:
            pass

    def timing(self, name: str, ms: float, attrs: dict[str, str] | None = None) -> None:
        try:
            _get_histogram(self._meter_name, name, "ms").record(ms, attrs or {})
        except Exception:
            pass

    def start_timer(self, name: str, attrs: dict[str, str] | None = None) -> Callable[[], None]:
        """Start a wall-clock timer; call the returned fn to record elapsed ms."""
        start = time.monotonic()

        def stop() -> None:
            ms = (time.monotonic() - start) * 1000.0
            try:
                _get_histogram(self._meter_name, name, "ms").record(ms, attrs or {})
            except Exception:
                pass

        return stop

    @contextmanager
    def with_timing(self, name: str, attrs: dict[str, str] | None = None) -> Iterator[None]:
        """Context manager that records elapsed ms with ``status=success|error``.

        TS exposes ``withTiming(name, fn)`` taking an async callable; the Pythonic
        equivalent is a context manager so it works for both sync and async
        bodies (``with``/``async with``-friendly via the timing being wall-clock)."""
        start = time.monotonic()
        status = "success"
        try:
            yield
        except Exception:
            status = "error"
            raise
        finally:
            ms = (time.monotonic() - start) * 1000.0
            merged = {**(attrs or {}), "status": status}
            try:
                _get_histogram(self._meter_name, name, "ms").record(ms, merged)
            except Exception:
                pass


def get_metrics_client(meter_name: str = DEFAULT_METER_NAME) -> MetricsClient:
    """Build a metrics client bound to a service-named meter (cheap)."""
    return MetricsClient(meter_name)


def _reset_metrics_instrument_cache_for_tests() -> None:
    _counter_cache.clear()
    _histogram_cache.clear()
