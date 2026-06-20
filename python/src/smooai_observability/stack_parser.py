"""Python traceback → structured ``StackFrame`` list.

The TS SDK parses a JS ``Error.stack`` *string* (V8 / Gecko / JSC formats).
Python doesn't have that problem — the ``traceback`` module gives us structured
frames directly — so this module walks ``traceback.extract_tb`` instead of
regex-parsing a string.

Frames are returned **innermost-first** to match the ``@smooai/observability``
event envelope (TS ``parseStack`` returns innermost-first too). Python's
``extract_tb`` yields outermost-first, so we reverse.

A frame is tagged ``in_app = False`` when it lives in the stdlib, in
``site-packages`` / ``dist-packages`` (third-party deps), or inside this SDK —
mirroring the TS ``node_modules`` + SDK-internal tagging.
"""

from __future__ import annotations

import os
import sys
import sysconfig
import traceback
from types import TracebackType

from .types import StackFrame

# Path fragments that mark a frame as NOT application code. Mirrors the TS
# NODE_MODULES_RE + SDK_INTERNAL_HINTS split, adapted to Python's layout.
_VENDOR_HINTS = (
    f"{os.sep}site-packages{os.sep}",
    f"{os.sep}dist-packages{os.sep}",
)
_SDK_INTERNAL_HINTS = (
    f"{os.sep}smooai_observability{os.sep}",
    "smooai_observability/",
)


def _stdlib_dirs() -> tuple[str, ...]:
    dirs: set[str] = set()
    for key in ("stdlib", "platstdlib"):
        try:
            p = sysconfig.get_path(key)
        except (KeyError, OSError):
            p = None
        if p:
            dirs.add(os.path.normpath(p))
    # sys.prefix/lib catches some layouts sysconfig misses.
    dirs.add(os.path.normpath(os.path.join(sys.prefix, "lib")))
    return tuple(dirs)


_STDLIB_DIRS = _stdlib_dirs()


def _is_in_app(filename: str) -> bool:
    norm = os.path.normpath(filename)
    if any(h in norm for h in _SDK_INTERNAL_HINTS):
        return False
    if any(h in norm for h in _VENDOR_HINTS):
        return False
    if any(norm.startswith(d + os.sep) for d in _STDLIB_DIRS):
        return False
    return True


def _is_sdk_internal(filename: str) -> bool:
    norm = os.path.normpath(filename)
    return any(h in norm for h in _SDK_INTERNAL_HINTS)


def parse_traceback(tb: TracebackType | None) -> list[StackFrame]:
    """Extract structured frames from a traceback, innermost-first."""
    if tb is None:
        return []
    extracted = traceback.extract_tb(tb)  # outermost-first
    frames: list[StackFrame] = []
    for fs in extracted:
        frames.append(
            StackFrame(
                module=fs.filename,
                function=fs.name or None,
                lineno=fs.lineno,
                colno=getattr(fs, "colno", None),
                in_app=_is_in_app(fs.filename),
            )
        )
    frames.reverse()  # innermost-first to match the envelope
    return frames


def parse_current_stack(skip: int = 0) -> list[StackFrame]:
    """Snapshot the *current* call stack (for ``capture_message``), innermost
    first. ``skip`` drops that many innermost SDK frames so the reported top is
    the caller, not the SDK plumbing."""
    summary = traceback.extract_stack()  # outermost-first, includes this fn
    frames: list[StackFrame] = []
    for fs in summary:
        frames.append(
            StackFrame(
                module=fs.filename,
                function=fs.name or None,
                lineno=fs.lineno,
                colno=getattr(fs, "colno", None),
                in_app=_is_in_app(fs.filename),
            )
        )
    frames.reverse()  # innermost-first
    return drop_sdk_frames(frames)[skip:] if skip else drop_sdk_frames(frames)


def drop_sdk_frames(frames: list[StackFrame]) -> list[StackFrame]:
    """Strip SDK-internal frames from the top (innermost) of a stack. Mirrors
    TS ``dropSdkFrames``."""
    i = 0
    while i < len(frames) and frames[i].in_app is False and _is_sdk_internal(frames[i].module):
        i += 1
    return frames[i:]
