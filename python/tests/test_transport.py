import threading
import time

import httpx

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


class _RecordingTransport:
    """Captures POSTed payloads via an httpx MockTransport."""

    def __init__(self):
        self.payloads: list[dict] = []
        self.lock = threading.Lock()
        self.event = threading.Event()

        def handler(request: httpx.Request) -> httpx.Response:
            with self.lock:
                import json

                self.payloads.append(json.loads(request.content))
            self.event.set()
            return httpx.Response(200, json={"ok": True})

        self.client = httpx.Client(transport=httpx.MockTransport(handler))


def test_transport_batches_and_posts():
    rec = _RecordingTransport()
    t = Transport(
        "https://example.test/webhook",
        client=rec.client,
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
            all_events = [e for p in rec.payloads for e in p["events"]]
        assert len(all_events) == 3
        assert rec.payloads[0]["type"] == "error"
    finally:
        t.shutdown()


def test_transport_flush_on_full_batch():
    rec = _RecordingTransport()
    t = Transport(
        "https://example.test/webhook",
        client=rec.client,
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
    rec = _RecordingTransport()
    t = Transport(
        "https://example.test/webhook",
        client=rec.client,
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
    state = {"calls": 0}
    flushed = threading.Event()

    def handler(request: httpx.Request) -> httpx.Response:
        state["calls"] += 1
        if state["calls"] == 1:
            return httpx.Response(500)
        flushed.set()
        return httpx.Response(200)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    t = Transport("https://example.test/webhook", client=client, flush_interval_ms=50, max_batch_size=100)
    try:
        t.enqueue(_event(0))
        assert flushed.wait(3.0), "retry never succeeded"
        assert state["calls"] >= 2
    finally:
        t.shutdown()
