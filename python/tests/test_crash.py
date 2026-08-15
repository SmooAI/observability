"""Crash-handler tests — every one of these runs a REAL subprocess that really
dies, because that is the only thing the feature is about.

Calling ``sys.excepthook`` by hand proves nothing: not that it is installed, not
that the previous hook survives, and above all not that the batched exporters are
flushed before the interpreter goes away. So each test writes a child program,
runs it with ``subprocess.run``, and asserts on three things the parent can still
see afterwards: the exit status, stderr, and the JSONL files that the child's
span/log exporters wrote to disk.

The child's processors are deliberately configured with a 60s schedule delay and
``shutdown_on_exit=False``, so a span/log file that has content can ONLY have got
there via the crash handler's force-flush. Remove the flush and these files stay
empty — which is exactly the mutation check.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path

# Exporters that survive the process: they append JSONL to a file the parent
# reads after the child is dead. Batched with a 60s delay so only an explicit
# force_flush can produce output; shutdown_on_exit=False so atexit can't do it
# for us and mask a missing flush.
_PREAMBLE = """
import json, sys, threading
from opentelemetry import trace
from opentelemetry._logs import set_logger_provider
from opentelemetry.sdk._logs import LoggerProvider, LoggingHandler
from opentelemetry.sdk._logs.export import BatchLogRecordProcessor, LogExporter, LogExportResult
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor, SpanExporter, SpanExportResult
import logging

SPANS = {spans!r}
LOGS = {logs!r}

class FileSpanExporter(SpanExporter):
    def export(self, spans):
        with open(SPANS, "a") as f:
            for s in spans:
                f.write(json.dumps({{
                    "name": s.name,
                    "status": s.status.status_code.name,
                    "attributes": {{k: str(v) for k, v in (s.attributes or {{}}).items()}},
                    "events": [
                        {{"name": e.name, "attributes": {{k: str(v) for k, v in (e.attributes or {{}}).items()}}}}
                        for e in s.events
                    ],
                }}) + "\\n")
        return SpanExportResult.SUCCESS
    def shutdown(self): pass
    def force_flush(self, timeout_millis=30000): return True

class FileLogExporter(LogExporter):
    def export(self, batch):
        with open(LOGS, "a") as f:
            for r in batch:
                rec = r.log_record
                f.write(json.dumps({{
                    "severity": rec.severity_text,
                    "body": str(rec.body),
                    "attributes": {{k: str(v) for k, v in (rec.attributes or {{}}).items()}},
                }}) + "\\n")
        return LogExportResult.SUCCESS
    def shutdown(self): pass
    def force_flush(self, timeout_millis=30000): return True

tp = TracerProvider(shutdown_on_exit=False)
tp.add_span_processor(BatchSpanProcessor(FileSpanExporter(), schedule_delay_millis=60000))
trace.set_tracer_provider(tp)

lp = LoggerProvider(shutdown_on_exit=False)
lp.add_log_record_processor(BatchLogRecordProcessor(FileLogExporter(), schedule_delay_millis=60000))
set_logger_provider(lp)
logging.getLogger().addHandler(LoggingHandler(level=logging.NOTSET, logger_provider=lp))
logging.getLogger().setLevel(logging.INFO)

from smooai_observability import install_crash_handler
from smooai_observability.client import Client, ClientOptions
from smooai_observability.otel import register_otel_capture

Client.init(ClientOptions(environment="test", release="1.2.3"))
register_otel_capture()
"""


def _run(tmp_path: Path, body: str, timeout: float = 60.0) -> tuple[subprocess.CompletedProcess[str], list[dict], list[dict], float]:
    spans = tmp_path / "spans.jsonl"
    logs = tmp_path / "logs.jsonl"
    script = _PREAMBLE.format(spans=str(spans), logs=str(logs)) + body
    started = time.monotonic()
    proc = subprocess.run(
        [sys.executable, "-c", script],
        capture_output=True,
        text=True,
        timeout=timeout,
        # NO_COLOR: 3.13+ colorizes tracebacks with ANSI escapes, which would
        # break the plain-substring assertions on stderr below.
        env={**os.environ, "PYTHONUNBUFFERED": "1", "NO_COLOR": "1"},
    )
    elapsed = time.monotonic() - started
    return proc, _read(spans), _read(logs), elapsed


def _read(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def _exception_spans(spans: list[dict]) -> list[dict]:
    return [s for s in spans if any(e["name"] == "exception" for e in s["events"])]


def test_main_thread_crash_exports_exception_span_and_fatal_log(tmp_path: Path) -> None:
    proc, spans, logs, _ = _run(
        tmp_path,
        """
install_crash_handler()
raise RuntimeError("boom-main")
""",
    )

    assert proc.returncode == 1, proc.stderr
    # The interpreter's own traceback must survive — a reporter that hides it is
    # worse than no reporter.
    assert "Traceback (most recent call last)" in proc.stderr
    assert "RuntimeError: boom-main" in proc.stderr

    exc_spans = _exception_spans(spans)
    assert len(exc_spans) == 1, spans
    span = exc_spans[0]
    assert span["status"] == "ERROR"
    event = next(e for e in span["events"] if e["name"] == "exception")
    assert event["attributes"]["exception.type"] == "RuntimeError"
    assert "boom-main" in event["attributes"]["exception.message"]
    assert span["attributes"]["smoo.tag.source"] == "sys.excepthook"

    fatal = [r for r in logs if r["severity"] in ("CRITICAL", "FATAL")]
    assert fatal, logs
    assert any("boom-main" in r["body"] for r in fatal)
    assert any(r["attributes"].get("exception.type") == "RuntimeError" for r in fatal)


def test_worker_thread_crash_is_reported(tmp_path: Path) -> None:
    proc, spans, logs, _ = _run(
        tmp_path,
        """
install_crash_handler()

def work():
    raise ValueError("boom-worker")

t = threading.Thread(target=work, name="worker-1")
t.start()
t.join()
print("main survived")
""",
    )

    # A thread crash doesn't kill the process — but it is still a crash.
    assert proc.returncode == 0, proc.stderr
    assert "main survived" in proc.stdout
    assert "Exception in thread worker-1" in proc.stderr
    assert "ValueError: boom-worker" in proc.stderr

    exc_spans = _exception_spans(spans)
    assert len(exc_spans) == 1, spans
    span = exc_spans[0]
    assert span["status"] == "ERROR"
    assert span["attributes"]["smoo.tag.source"] == "threading.excepthook"
    assert span["attributes"]["smoo.tag.thread"] == "worker-1"
    assert any("boom-worker" in r["body"] for r in logs if r["severity"] in ("CRITICAL", "FATAL")), logs


def test_previous_hooks_still_run(tmp_path: Path) -> None:
    proc, spans, _, _ = _run(
        tmp_path,
        """
prev_sys = sys.excepthook
prev_thread = threading.excepthook

def my_sys_hook(t, v, tb):
    print("PREV-SYS-HOOK-RAN", file=sys.stderr)
    prev_sys(t, v, tb)

def my_thread_hook(args):
    print("PREV-THREAD-HOOK-RAN", file=sys.stderr)
    prev_thread(args)

sys.excepthook = my_sys_hook
threading.excepthook = my_thread_hook

install_crash_handler()

t = threading.Thread(target=lambda: (_ for _ in ()).throw(ValueError("boom-worker")), name="worker-1")
t.start()
t.join()

raise RuntimeError("boom-main")
""",
    )

    assert proc.returncode == 1, proc.stderr
    assert "PREV-SYS-HOOK-RAN" in proc.stderr
    assert "PREV-THREAD-HOOK-RAN" in proc.stderr
    # The chained hook must not have cost us the report, or the traceback.
    assert "RuntimeError: boom-main" in proc.stderr
    assert len(_exception_spans(spans)) == 2, spans


def test_double_install_reports_once(tmp_path: Path) -> None:
    proc, spans, logs, _ = _run(
        tmp_path,
        """
install_crash_handler()
install_crash_handler()
install_crash_handler()
raise RuntimeError("boom-main")
""",
    )

    assert proc.returncode == 1, proc.stderr
    assert len(_exception_spans(spans)) == 1, spans
    assert len([r for r in logs if r["severity"] in ("CRITICAL", "FATAL")]) == 1, logs
    # A chain that grew per install would print the traceback once per link.
    assert proc.stderr.count("RuntimeError: boom-main") == 1, proc.stderr


def test_reporter_failure_does_not_suppress_original_traceback(tmp_path: Path) -> None:
    proc, _, _, _ = _run(
        tmp_path,
        """
def explode(*a, **k):
    raise OSError("reporter is broken")

Client.capture_exception = explode
install_crash_handler()
raise RuntimeError("boom-main")
""",
    )

    assert proc.returncode == 1, proc.stderr
    assert "RuntimeError: boom-main" in proc.stderr
    # The reporter's own failure must not replace or shadow the real crash.
    assert "reporter is broken" not in proc.stderr


def test_hung_flush_cannot_block_process_exit(tmp_path: Path) -> None:
    proc, _, _, elapsed = _run(
        tmp_path,
        """
import time
install_crash_handler(flush=lambda _t: time.sleep(120))
raise RuntimeError("boom-main")
""",
        timeout=60.0,
    )

    assert proc.returncode == 1, proc.stderr
    assert "RuntimeError: boom-main" in proc.stderr
    # A crash handler that keeps a dying process alive is a worse bug than the
    # report it was trying to send. 2s budget + interpreter shutdown, generously
    # bounded here so a loaded machine doesn't flake it.
    assert elapsed < 20, f"crash handler blocked exit for {elapsed:.1f}s"


def test_bootstrap_installs_the_crash_handler(tmp_path: Path) -> None:
    # The wiring test: nothing in the child touches crash.py directly.
    proc, _, _, _ = _run(
        tmp_path,
        """
before = sys.excepthook
from smooai_observability import bootstrap_observability
bootstrap_observability(fetch_token=False)
print("SYS", sys.excepthook is not before)
print("THREAD", threading.excepthook is not threading.__excepthook__)
""",
    )

    assert proc.returncode == 0, proc.stderr
    assert "SYS True" in proc.stdout
    assert "THREAD True" in proc.stdout
