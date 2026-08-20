"""ADR-097 — session-scoped sampling. Python port of ``packages/core/src/sampling.ts``.

THE CORE RULE: the sampling decision is made ONCE per session (or per trace,
where one exists) and applies to EVERY log line under it. The invariant this
buys: **any trace you can open has 100% of its log lines**. Never a partial view.

Every vector this module must reproduce lives in ``parity/sampling-corpus.json``
and is asserted by ``tests/test_parity_corpus.py`` — the same file the TS, Rust,
Go and .NET lanes load. See ``parity/README.md`` for the hash derivation and the
porting traps.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from enum import StrEnum

_FNV_OFFSET_BASIS_32 = 0x811C9DC5
_FNV_PRIME_32 = 0x01000193
_MASK_32 = 0xFFFFFFFF
# 2^32 as an exact binary64 — dividing a u32 by this is a pure exponent
# adjustment, so every language gets the identical double.
_TWO_POW_32 = 4294967296.0


def fnv1a32(value: str) -> int:
    """FNV-1a 32-bit over the UTF-8 bytes of ``value``. Returns an unsigned int.

    XOR-then-multiply (this is FNV-*1a*, not FNV-1), masked to 32 bits after
    each step because Python ints are arbitrary precision and would otherwise
    never wrap.
    """
    h = _FNV_OFFSET_BASIS_32
    for b in value.encode("utf-8"):
        h ^= b
        h = (h * _FNV_PRIME_32) & _MASK_32
    return h


def sample_decision(id: str, ratio: float) -> bool:  # noqa: A002 - `id` matches the cross-SDK parameter name
    """The one sampling primitive. Deterministic and stable for an id's lifetime.

    * non-finite ``ratio`` → IN (fail open — telemetry going dark on a config
      hiccup is the failure ADR-097 forbids; ``x < NaN`` is false everywhere, so
      the naive path would sample *everything* out)
    * ``ratio <= 0.0`` → OUT, ``ratio >= 1.0`` → IN, both taken before any float
      math so 1.0 can never drop and 0.0 can never keep
    * otherwise ``(hash / 2**32) < ratio``, strict less-than
    """
    if not math.isfinite(ratio):
        return True
    if ratio <= 0.0:
        return False
    if ratio >= 1.0:
        return True
    return fnv1a32(id) / _TWO_POW_32 < ratio


class CanonicalLevel(StrEnum):
    """Canonical log levels, uppercase (ADR-096).

    Distinct from :data:`smooai_observability.types.Level`, which is the
    lowercase *event wire* severity on the ingest envelope. This is the log-line
    severity the ClickHouse queries filter on, and ``level IN ('ERROR','FATAL')``
    is CASE-SENSITIVE — emitting ``"error"`` silently makes every error a
    non-error.
    """

    TRACE = "TRACE"
    DEBUG = "DEBUG"
    INFO = "INFO"
    WARN = "WARN"
    ERROR = "ERROR"
    FATAL = "FATAL"


#: Ordering used by the minimum-level filter. Matches OTel severity numbers.
LEVEL_RANK: dict[CanonicalLevel, int] = {
    CanonicalLevel.TRACE: 1,
    CanonicalLevel.DEBUG: 5,
    CanonicalLevel.INFO: 9,
    CanonicalLevel.WARN: 13,
    CanonicalLevel.ERROR: 17,
    CanonicalLevel.FATAL: 21,
}

#: Aliases every SDK must accept. Part of the parity contract.
_LEVEL_ALIASES: dict[str, CanonicalLevel] = {
    "trace": CanonicalLevel.TRACE,
    "verbose": CanonicalLevel.TRACE,
    "debug": CanonicalLevel.DEBUG,
    "info": CanonicalLevel.INFO,
    "information": CanonicalLevel.INFO,
    "log": CanonicalLevel.INFO,
    "notice": CanonicalLevel.INFO,
    "warn": CanonicalLevel.WARN,
    "warning": CanonicalLevel.WARN,
    "error": CanonicalLevel.ERROR,
    "err": CanonicalLevel.ERROR,
    "fatal": CanonicalLevel.FATAL,
    "critical": CanonicalLevel.FATAL,
    "crit": CanonicalLevel.FATAL,
    "emergency": CanonicalLevel.FATAL,
    "panic": CanonicalLevel.FATAL,
}


def parse_level(level: str) -> CanonicalLevel | None:
    """Strict parse: a known level spelling, or ``None``."""
    return _LEVEL_ALIASES.get(level.strip().lower())


def normalize_level(level: str) -> CanonicalLevel:
    """Normalize any level spelling to the canonical form.

    Unknown spellings become ``INFO`` — fail-safe: an unrecognised level must
    never cause a drop, and must never be promoted into ERROR (which would
    corrupt the error rate).
    """
    return parse_level(level) or CanonicalLevel.INFO


def meets_minimum_level(level: CanonicalLevel, minimum: CanonicalLevel) -> bool:
    """True when ``level`` is at or above ``minimum``."""
    return LEVEL_RANK[level] >= LEVEL_RANK[minimum]


@dataclass(frozen=True)
class LogSamplingInput:
    """Input to :func:`should_emit_log`."""

    #: Level as emitted by the caller; normalized internally.
    level: str
    #: Stable per-page session id. Used when no trace context exists.
    session_id: str
    #: Kill switch — ``False`` disables all telemetry emission, errors included.
    enabled: bool
    #: Minimum level to emit.
    minimum_level: CanonicalLevel
    #: Session-scoped browser log sampling ratio.
    log_sampling_ratio: float
    #: The trace's own sampling decision, when a trace context exists. Where a
    #: trace exists its decision WINS, so spans and logs never disagree.
    trace_sampled: bool | None = None


def should_emit_log(input: LogSamplingInput) -> bool:  # noqa: A002 - mirrors the TS parameter name
    """The single decision point for "does this log line get emitted?".

    Order matters and is part of the parity contract:

    1. kill switch — off means off, no exceptions
    2. minimum level — below the floor is not emitted
    3. WARN/ERROR/FATAL — always 100% (ADR-010: "sampling errors is malpractice")
    4. trace decision, if a trace exists — inherited, never re-rolled
    5. otherwise the session decision — one roll for the whole session
    """
    if not input.enabled:
        return False
    level = normalize_level(input.level)
    if not meets_minimum_level(level, input.minimum_level):
        return False
    if LEVEL_RANK[level] >= LEVEL_RANK[CanonicalLevel.WARN]:
        return True
    if input.trace_sampled is not None:
        return input.trace_sampled
    return sample_decision(input.session_id, input.log_sampling_ratio)
