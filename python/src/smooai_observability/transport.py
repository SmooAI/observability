"""Batched HTTP transport — ported from ``packages/core/src/transport.ts``.

Holds a small queue, flushes on a timer or when ``max_batch_size`` events are
buffered, drops the oldest event when the queue overflows, and pushes a failed
batch back to the front for the next attempt. POSTs the same
``{type: 'error', events: [...]}`` envelope the TS SDK sends to the DSN webhook.

The webhook POST goes through ``smooai-fetch`` (timeouts + retries + circuit
breaking) rather than raw ``httpx`` (SMOODEV-2026). smooai-fetch already retries
429/5xx and network/timeout errors internally; the queue requeue here covers the
post-retry surface so a permanently-failing endpoint still re-tries on the next
flush tick.

The TS version runs on the JS event loop (``setTimeout`` + ``fetch``). Python has
no implicit loop, and the SDK must work in plain sync hosts (Lambda handlers,
scripts) as well as async ones, so this uses a daemon worker thread with a flush
interval — no asyncio dependency on the host. ``smooai-fetch`` is async-only, so
the worker thread owns a private event loop and drives ``fetch()`` via
``run_until_complete``; the host's loop (if any) is never touched. Errors are
swallowed: observability must never throw into user code.
"""

from __future__ import annotations

import asyncio
import threading
from collections.abc import Awaitable, Callable
from typing import Any

from smooai_fetch import FetchOptions, fetch
from smooai_fetch._types import RetryOptions, TimeoutOptions

from .types import IngestPayload, ObservabilityEvent

DEFAULT_FLUSH_INTERVAL_MS = 1000
DEFAULT_BATCH_SIZE = 30
DEFAULT_QUEUE_MAX = 250

# Type of the injectable fetch callable. Mirrors the subset of ``smooai_fetch.fetch``
# the transport uses: ``await fetch_fn(url, options)``. Tests substitute a stub.
FetchFn = Callable[[str, FetchOptions], Awaitable[Any]]


class Transport:
    """Thread-backed batched transport.

    The webhook POST is delegated to ``smooai-fetch`` (``fetch_fn``), which owns
    retries/timeouts/circuit-breaking. ``fetch_fn`` is injectable so tests can
    substitute a stub — ``smooai-fetch`` constructs its own
    ``httpx.AsyncClient`` internally and exposes no client-injection seam.
    """

    def __init__(
        self,
        dsn: str,
        *,
        flush_interval_ms: int = DEFAULT_FLUSH_INTERVAL_MS,
        max_batch_size: int = DEFAULT_BATCH_SIZE,
        max_queue_size: int = DEFAULT_QUEUE_MAX,
        fetch_fn: FetchFn | None = None,
        timeout_s: float = 5.0,
    ) -> None:
        self._dsn = dsn
        self._flush_interval_s = max(flush_interval_ms, 0) / 1000.0
        self._max_batch_size = max(max_batch_size, 1)
        self._max_queue_size = max(max_queue_size, 1)
        self._timeout_s = timeout_s
        # Caller may inject a fetch fn (tests); otherwise use smooai-fetch.
        self._fetch_fn: FetchFn = fetch_fn or fetch

        self._queue: list[ObservabilityEvent] = []
        self._lock = threading.Lock()
        self._wake = threading.Event()
        self._stopped = False
        # Private loop owned by the worker thread; smooai-fetch needs to run on
        # an event loop and the host's loop must never be touched.
        self._loop = asyncio.new_event_loop()
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
        events aren't lost when the process exits. Runs the async send on the
        worker loop so there is a single owner for the smooai-fetch event loop."""
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
        asyncio.set_event_loop(self._loop)
        try:
            while True:
                # Wake early when a full batch lands; otherwise tick on the interval.
                self._wake.wait(self._flush_interval_s)
                self._wake.clear()
                if self._drain_once() == "stop":
                    return
        finally:
            try:
                self._loop.close()
            except Exception:
                pass

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
        try:
            self._run_async(self._send_async(batch))
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

    def _run_async(self, coro: Awaitable[Any]) -> None:
        """Drive a coroutine to completion on the right loop.

        The worker thread owns ``self._loop`` and reuses it (cheap, keeps a
        single owner for the smooai-fetch loop). ``flush()`` may also be called
        from the host thread at shutdown — a loop must not be driven from two
        threads, so off-worker callers get a transient loop via ``asyncio.run``."""
        if threading.current_thread() is self._thread:
            self._loop.run_until_complete(coro)
        else:
            asyncio.run(coro)

    async def _send_async(self, batch: list[ObservabilityEvent]) -> None:
        """POST one batch via smooai-fetch. smooai-fetch raises on non-2xx (after
        its own 429/5xx retries) and on transport/timeout errors — exactly the
        cases the caller requeues."""
        payload = IngestPayload(events=batch)
        await self._fetch_fn(
            self._dsn,
            FetchOptions(
                method="POST",
                headers={"content-type": "application/json"},
                body=payload.to_wire(),
                # smooai-fetch's default retry covers 429/5xx + network errors;
                # cap the per-request timeout so a flush can't hang the worker.
                retry=RetryOptions(),
                timeout=TimeoutOptions(timeout_ms=self._timeout_s * 1000.0),
            ),
        )

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
