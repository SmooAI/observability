import threading
import time

import httpx
from smooai_fetch import FetchOptions, FetchResponse
from smooai_fetch._errors import HTTPResponseError

from smooai_observability.transport import Transport
from smooai_observability.types import ObservabilityEvent, Sdk


def _event(i: int) -> ObservabilityEvent:
    return ObservabilityEvent(
        event_id=f"id-{i}",
        timestamp=i,
        level="error",
        message=f"m{i}",
        sdk=Sdk("@smooai/observability", "0.1.0", "python"),
    )


def _ok_response(url: str) -> FetchResponse:
    """A minimal successful FetchResponse, as smooai-fetch would return."""
    return FetchResponse(response=httpx.Response(200, request=httpx.Request("POST", url)))


class _RecordingFetch:
    """Captures the bodies POSTed through an injected smooai-fetch stub.

    smooai-fetch constructs its own ``httpx.AsyncClient`` internally and exposes
    no client-injection seam, so the transport takes an injectable ``fetch_fn``
    and tests substitute this stub.
    """

    def __init__(self):
        self.bodies: list[dict] = []
        self.lock = threading.Lock()
        self.event = threading.Event()

    async def __call__(self, url: str, options: FetchOptions) -> FetchResponse:
        with self.lock:
            self.bodies.append(options.body)
        self.event.set()
        return _ok_response(url)


def test_transport_batches_and_posts():
    rec = _RecordingFetch()
    t = Transport(
        "https://example.test/webhook",
        fetch_fn=rec,
        flush_interval_ms=50,
        max_batch_size=5,
    )
    try:
        for i in range(3):
            t.enqueue(_event(i))
        # Wait for the timer-driven flush.
        assert rec.event.wait(2.0), "transport never flushed"
        time.sleep(0.1)
        with rec.lock:
            all_events = [e for body in rec.bodies for e in body["events"]]
        assert len(all_events) == 3
        assert rec.bodies[0]["type"] == "error"
    finally:
        t.shutdown()


def test_transport_flush_on_full_batch():
    rec = _RecordingFetch()
    t = Transport(
        "https://example.test/webhook",
        fetch_fn=rec,
        flush_interval_ms=10_000,  # long, so only batch-size triggers flush
        max_batch_size=3,
    )
    try:
        for i in range(3):
            t.enqueue(_event(i))
        assert rec.event.wait(2.0), "full batch did not trigger flush"
    finally:
        t.shutdown()


def test_transport_drops_oldest_on_overflow():
    rec = _RecordingFetch()
    t = Transport(
        "https://example.test/webhook",
        fetch_fn=rec,
        flush_interval_ms=10_000,
        max_batch_size=1000,
        max_queue_size=5,
    )
    try:
        for i in range(10):
            t.enqueue(_event(i))
        assert t.queue_size() <= 5
    finally:
        t.shutdown()


def test_transport_retries_on_failure():
    """smooai-fetch raises on non-2xx (after its own retries); the transport
    requeues the failed batch and re-sends it on the next flush tick."""
    state = {"calls": 0}
    flushed = threading.Event()
    lock = threading.Lock()

    async def fetch_fn(url: str, options: FetchOptions) -> FetchResponse:
        with lock:
            state["calls"] += 1
            n = state["calls"]
        if n == 1:
            # Mirror smooai-fetch surfacing a non-2xx after exhausting retries.
            raise HTTPResponseError(httpx.Response(500, request=httpx.Request("POST", url)))
        flushed.set()
        return _ok_response(url)

    t = Transport("https://example.test/webhook", fetch_fn=fetch_fn, flush_interval_ms=50, max_batch_size=100)
    try:
        t.enqueue(_event(0))
        assert flushed.wait(3.0), "retry never succeeded"
        with lock:
            assert state["calls"] >= 2
    finally:
        t.shutdown()
