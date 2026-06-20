"""Batched HTTP transport — ported from ``packages/core/src/transport.ts``.

Holds a small queue, flushes on a timer or when ``max_batch_size`` events are
buffered, drops the oldest event when the queue overflows, and pushes a failed
batch back to the front for the next attempt. POSTs the same
``{type: 'error', events: [...]}`` envelope the TS SDK sends to the DSN webhook.

The TS version runs on the JS event loop (``setTimeout`` + ``fetch``). Python
has no implicit loop, and the SDK must work in plain sync hosts (Lambda
handlers, scripts) as well as async ones, so this uses a daemon worker thread
with a flush interval — no asyncio dependency, never blocks the host. Errors are
swallowed: observability must never throw into user code.
"""

from __future__ import annotations

import threading

import httpx

from .types import IngestPayload, ObservabilityEvent

DEFAULT_FLUSH_INTERVAL_MS = 1000
DEFAULT_BATCH_SIZE = 30
DEFAULT_QUEUE_MAX = 250


class Transport:
    """Thread-backed batched transport."""

    def __init__(
        self,
        dsn: str,
        *,
        flush_interval_ms: int = DEFAULT_FLUSH_INTERVAL_MS,
        max_batch_size: int = DEFAULT_BATCH_SIZE,
        max_queue_size: int = DEFAULT_QUEUE_MAX,
        client: httpx.Client | None = None,
        timeout_s: float = 5.0,
    ) -> None:
        self._dsn = dsn
        self._flush_interval_s = max(flush_interval_ms, 0) / 1000.0
        self._max_batch_size = max(max_batch_size, 1)
        self._max_queue_size = max(max_queue_size, 1)
        self._timeout_s = timeout_s
        # Caller may inject a client (tests / connection reuse); otherwise own one.
        self._owns_client = client is None
        self._client = client or httpx.Client(timeout=timeout_s)

        self._queue: list[ObservabilityEvent] = []
        self._lock = threading.Lock()
        self._wake = threading.Event()
        self._stopped = False
        self._thread = threading.Thread(
            target=self._run,
            name="smooai-observability-transport",
            daemon=True,
        )
        self._thread.start()

    def enqueue(self, event: ObservabilityEvent) -> None:
        """Add an event to the queue. Drops the oldest event when full (recent
        events are more useful), and wakes the worker when a full batch is
        ready."""
        with self._lock:
            if self._stopped:
                return
            if len(self._queue) >= self._max_queue_size:
                self._queue.pop(0)  # drop oldest
            self._queue.append(event)
            ready = len(self._queue) >= self._max_batch_size
        if ready:
            self._wake.set()

    def flush(self, timeout_s: float | None = None) -> None:
        """Synchronously drain the queue. Used at shutdown / SIGTERM so buffered
        events aren't lost when the process exits."""
        deadline_iter = True
        while deadline_iter:
            with self._lock:
                batch = self._queue[: self._max_batch_size]
                del self._queue[: len(batch)]
                deadline_iter = len(self._queue) > 0
            if not batch:
                break
            self._send(batch)

    def _run(self) -> None:
        while True:
            # Wake early when a full batch lands; otherwise tick on the interval.
            self._wake.wait(self._flush_interval_s)
            self._wake.clear()
            if self._drain_once() == "stop":
                return

    def _drain_once(self) -> str:
        with self._lock:
            stopped = self._stopped
            batch = self._queue[: self._max_batch_size]
            del self._queue[: len(batch)]
        if batch:
            self._send(batch)
        if stopped:
            with self._lock:
                remaining = bool(self._queue)
            if not remaining:
                return "stop"
        return "continue"

    def _send(self, batch: list[ObservabilityEvent]) -> None:
        payload = IngestPayload(events=batch)
        try:
            resp = self._client.post(
                self._dsn,
                json=payload.to_wire(),
                headers={"content-type": "application/json"},
            )
            if resp.status_code >= 400:
                raise httpx.HTTPStatusError("ingest rejected", request=resp.request, response=resp)
        except Exception:
            # Best-effort: push the batch back to the front for a retry. Never
            # raise — observability must not crash the host.
            with self._lock:
                if not self._stopped:
                    self._queue[0:0] = batch
                    # Re-cap to the queue limit (oldest-out) after re-insert.
                    if len(self._queue) > self._max_queue_size:
                        overflow = len(self._queue) - self._max_queue_size
                        del self._queue[:overflow]

    def queue_size(self) -> int:
        with self._lock:
            return len(self._queue)

    def shutdown(self, timeout_s: float = 2.0) -> None:
        """Stop the worker after draining outstanding events."""
        with self._lock:
            self._stopped = True
        self.flush()
        self._wake.set()
        self._thread.join(timeout=timeout_s)
        if self._owns_client:
            try:
                self._client.close()
            except Exception:
                pass
