"""W3C ``traceparent`` parse / format.

Python port of ``packages/core/src/traceparent.ts``, pinned by
``parity/sampling-corpus.json``::

    00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
    ^  ^                                ^                ^
    |  trace-id (16 bytes, 32 lowercase hex)             |
    |                                   span-id (8 bytes, 16 lowercase hex)
    version (1 byte, 2 hex)                              trace-flags (1 byte, 2 hex)

Parsing is STRICT — exactly four dash-separated fields, version exactly ``00``.
Rejected: wrong field count, wrong version (including the forbidden ``ff``),
non-hex or wrong-length fields, uppercase hex, an all-zero trace id, an all-zero
span id. The all-zero ids are the classic "propagated a placeholder" bug:
accepting them produces traces that all collide on ``000…0``.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

_VERSION = "00"
_HEX32 = re.compile(r"^[0-9a-f]{32}$")
_HEX16 = re.compile(r"^[0-9a-f]{16}$")
_HEX2 = re.compile(r"^[0-9a-f]{2}$")


@dataclass(frozen=True)
class TraceContext:
    """A parsed ``traceparent``."""

    #: 32 lowercase hex chars.
    trace_id: str
    #: 16 lowercase hex chars.
    span_id: str
    #: trace-flags byte, 0-255.
    flags: int
    #: bit 0 of ``flags`` — the upstream sampling decision, which logs inherit.
    sampled: bool


def parse_traceparent(header: str) -> TraceContext | None:
    """Parse a ``traceparent`` header. Returns ``None`` for anything invalid."""
    parts = header.split("-")
    if len(parts) != 4:
        return None
    version, trace_id, span_id, flags_hex = parts
    if version != _VERSION:
        return None
    if not _HEX32.match(trace_id) or trace_id == "0" * 32:
        return None
    if not _HEX16.match(span_id) or span_id == "0" * 16:
        return None
    if not _HEX2.match(flags_hex):
        return None
    flags = int(flags_hex, 16)
    return TraceContext(trace_id=trace_id, span_id=span_id, flags=flags, sampled=flags & 0x01 == 0x01)


def format_traceparent(trace_id: str, span_id: str, flags: int | None = None, sampled: bool | None = None) -> str | None:
    """Format a ``traceparent`` header.

    Pass ``flags`` explicitly, or let it be derived from ``sampled``. An
    out-of-byte-range ``flags`` is rejected rather than silently truncated.

    Returns ``None`` rather than emitting a header a spec-compliant peer would
    reject — a malformed traceparent breaks correlation downstream just as
    thoroughly as a missing one, but silently.
    """
    resolved = flags if flags is not None else (1 if sampled else 0)
    if not isinstance(resolved, int) or isinstance(resolved, bool) or not 0 <= resolved <= 255:
        return None
    header = f"{_VERSION}-{trace_id}-{span_id}-{resolved:02x}"
    # Round-trip through the parser so format can never emit what parse rejects.
    return header if parse_traceparent(header) else None
