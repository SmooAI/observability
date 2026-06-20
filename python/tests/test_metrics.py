"""Metrics tests.

OTel's global ``set_meter_provider`` may only be set ONCE per process (a second
call is ignored with a warning). So we install a single module-scoped
MeterProvider with an InMemoryMetricReader and read cumulatively, clearing the
SDK's instrument cache between tests. Each test uses uniquely-named instruments
so cumulative reads don't collide.
"""

from __future__ import annotations

import pytest
from opentelemetry import metrics as otel_metrics
from opentelemetry.sdk.metrics import MeterProvider
from opentelemetry.sdk.metrics.export import InMemoryMetricReader

from smooai_observability.metrics import (
    _reset_metrics_instrument_cache_for_tests,
    get_metrics_client,
)

_READER = InMemoryMetricReader()
_PROVIDER = MeterProvider(metric_readers=[_READER])
otel_metrics.set_meter_provider(_PROVIDER)


@pytest.fixture(autouse=True)
def _reset_cache():
    _reset_metrics_instrument_cache_for_tests()
    yield


def _all_metrics():
    data = _READER.get_metrics_data()
    out = []
    if data is None:
        return out
    for rm in data.resource_metrics:
        for sm in rm.scope_metrics:
            out.extend(sm.metrics)
    return out


def _by_name(name):
    return [m for m in _all_metrics() if m.name == name]


def test_counter_records():
    m = get_metrics_client("test-svc")
    m.counter("requests.total", 2, {"route": "/x"})
    m.counter("requests.total", 3, {"route": "/x"})
    counter = _by_name("requests.total")[0]
    points = list(counter.data.data_points)
    assert sum(p.value for p in points) == 5


def test_histogram_and_timing():
    m = get_metrics_client("test-svc")
    m.histogram("payload.size", 100)
    m.timing("latency.ms", 42)
    names = {x.name for x in _all_metrics()}
    assert "payload.size" in names
    assert "latency.ms" in names


def test_start_timer():
    m = get_metrics_client("test-svc")
    stop = m.start_timer("op.ms", {"op": "load"})
    stop()
    assert _by_name("op.ms"), "op.ms histogram not recorded"


def test_with_timing_records_status():
    m = get_metrics_client("test-svc")
    with m.with_timing("work.ms"):
        pass
    with pytest.raises(ValueError):
        with m.with_timing("work.ms"):
            raise ValueError("boom")
    hist = _by_name("work.ms")[0]
    statuses = {dp.attributes.get("status") for dp in hist.data.data_points}
    assert "success" in statuses
    assert "error" in statuses


def test_metrics_never_throw():
    # Even with weird input the client must not raise.
    m = get_metrics_client()
    m.counter("misc.count", 1, {"k": "v"})  # default meter name
