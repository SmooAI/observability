"""Unhandled-crash reporting — Python port of Rust ``install_panic_hook``
(``rust/observability/src/otel_capture.rs``) and TS
``registerNodeGlobalHandlers`` (``packages/core/src/node/global-handlers.ts``).

Without this a Python service can die from an uncaught exception — its single
loudest failure — and report nothing anywhere: the traceback goes to stderr, the
process exits, and the batched OTel exporters are killed with their queues still
full. Same hole the Rust panic hook exists to close.

What a crash produces, mirroring the Rust semantics:

  * a **CRITICAL stdlib log record** with ``exc_info``. Bootstrap attaches the
    OTel ``LoggingHandler`` to the ROOT logger, so this becomes an OTLP log
    record at severity FATAL carrying ``exception.*`` attributes — no separate
    log pipeline needed here.
  * a **``Client.capture_exception``**, which the registered OTel capture handler
    turns into a semconv ``exception`` span event with ``StatusCode.ERROR`` — on
    the active span if there is one, on a synthetic span otherwise.
  * a **bounded force-flush** of both, because everything above is batched and
    the process is about to be gone.

Three properties that are not optional:

  * **chains, never replaces.** The previously installed hook still runs, so the
    interpreter's own traceback still reaches stderr. A crash reporter that
    hides the traceback is worse than no crash reporter.
  * **idempotent.** A second install is a no-op; it does not grow the chain.
  * **never raises.** A failure inside the reporter must not mask or replace the
    exception that is killing the process.

Not handled here: asyncio "Task exception was never retrieved". That is not a
crash (the process survives), asyncio already reports it through the ``asyncio``
stdlib logger at ERROR with ``exc_info`` — which the root OTel LoggingHandler
already exports — and ``loop.set_exception_handler`` is per-loop, so it could
not be installed once at bootstrap for loops created later anyway. An exception
that escapes ``asyncio.run`` *is* a crash and reaches ``sys.excepthook`` here.
"""

from __future__ import annotations

import logging
import sys
import threading
from collections.abc import Callable
from types import TracebackType

from .client import Client

# Total wall-clock budget for the whole report (log + capture + flush). 2s
# matches ``OtelSdkHandle.flush``'s default and the TS SIGTERM flush hint, and is
# long enough for one OTLP round-trip on a healthy link. It is a HARD bound: the
# work runs on a daemon thread that is joined with this timeout, so a wedged
# exporter or a stalled token mint is abandoned rather than allowed to keep a
# dying process alive. A crash handler that prevents the process from exiting is
# a worse bug than the report it was trying to send.
CRASH_REPORT_TIMEOUT_S = 2.0

_logger = logging.getLogger("smooai.observability.crash")

_installed = False
_saved_hooks: tuple[Callable[..., None], Callable[..., None]] | None = None


def install_crash_handler(flush: Callable[[float], None] | None = None) -> None:
    """Install ``sys.excepthook`` + ``threading.excepthook`` crash reporting.

    Idempotent. ``flush`` is an optional extra drain hook (bootstrap wires the
    webhook transport into it) — OTel's tracer/logger providers are flushed
    unconditionally, so a caller that only uses OTel needs no argument.
    """
    global _installed, _saved_hooks
    if _installed:
        return
    _installed = True

    previous_sys_hook = sys.excepthook
    previous_thread_hook = threading.excepthook
    _saved_hooks = (previous_sys_hook, previous_thread_hook)

    def sys_hook(
        exc_type: type[BaseException],
        exc_value: BaseException,
        exc_tb: TracebackType | None,
    ) -> None:
        _report(exc_type, exc_value, exc_tb, threading.current_thread().name, "sys.excepthook", flush)
        previous_sys_hook(exc_type, exc_value, exc_tb)

    def thread_hook(args: threading.ExceptHookArgs) -> None:
        # A crash on a worker thread is still a crash — it just doesn't kill the
        # process. exc_value is None only for a thread killed at shutdown.
        if args.exc_value is not None:
            thread_name = args.thread.name if args.thread is not None else "unknown"
            _report(args.exc_type, args.exc_value, args.exc_traceback, thread_name, "threading.excepthook", flush)
        previous_thread_hook(args)

    sys.excepthook = sys_hook
    threading.excepthook = thread_hook


def _report(
    exc_type: type[BaseException],
    exc_value: BaseException,
    exc_tb: TracebackType | None,
    thread_name: str,
    source: str,
    flush: Callable[[float], None] | None,
) -> None:
    """Run the report under a hard time bound. Never raises."""
    try:
        worker = threading.Thread(
            target=_report_body,
            args=(exc_type, exc_value, exc_tb, thread_name, source, flush),
            name="smooai-observability-crash",
            daemon=True,
        )
        worker.start()
    except BaseException:
        # Can't spawn a thread during interpreter shutdown — report inline
        # instead. force_flush's own timeout is the bound on that path.
        try:
            _report_body(exc_type, exc_value, exc_tb, thread_name, source, flush)
        except BaseException:
            pass
        return
    try:
        # Separate try so an interrupted join falls through instead of
        # re-entering _report_body and reporting the same crash twice.
        worker.join(CRASH_REPORT_TIMEOUT_S)
    except BaseException:
        pass


def _report_body(
    exc_type: type[BaseException],
    exc_value: BaseException,
    exc_tb: TracebackType | None,
    thread_name: str,
    source: str,
    flush: Callable[[float], None] | None,
) -> None:
    # BaseException, not Exception: this runs as a thread target, and anything
    # escaping it would re-enter threading.excepthook — i.e. this handler.
    try:
        _logger.critical(
            "unhandled exception in thread %s: %s: %s",
            thread_name,
            exc_type.__name__,
            exc_value,
            exc_info=(exc_type, exc_value, exc_tb),
        )
        Client.capture_exception(exc_value, tags={"source": source, "thread": thread_name})
        if flush is not None:
            flush(CRASH_REPORT_TIMEOUT_S)
        _flush_otel(int(CRASH_REPORT_TIMEOUT_S * 1000))
    except BaseException:
        pass


def _flush_otel(timeout_millis: int) -> None:
    """Force-flush the global tracer + logger providers.

    Read from the OTel globals rather than from ``setup_otel_sdk``'s handle so a
    host that installed its own providers is flushed too. Each provider gets its
    own try — a failing trace flush must not skip the log flush.
    """
    from opentelemetry import trace
    from opentelemetry._logs import get_logger_provider

    providers = []
    for get in (trace.get_tracer_provider, get_logger_provider):
        try:
            providers.append(get())
        except Exception:
            pass
    for provider in providers:
        # The no-op/proxy providers have no force_flush; nothing to drain.
        force = getattr(provider, "force_flush", None)
        if not callable(force):
            continue
        try:
            force(timeout_millis)
        except Exception:
            pass


def reset_crash_handler_for_tests() -> None:
    """Test seam — restores the hooks that were in place before install."""
    global _installed, _saved_hooks
    if _saved_hooks is not None:
        sys.excepthook, threading.excepthook = _saved_hooks
        _saved_hooks = None
    _installed = False
