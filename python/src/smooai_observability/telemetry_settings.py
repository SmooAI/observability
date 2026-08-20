"""ADR-097 W1 — config-served telemetry settings.

Python port of ``packages/core/src/telemetry-settings.ts``, pinned by
``parity/sampling-corpus.json``.

These are ``@smooai/config`` **public-tier**, org-scoped keys. Public tier is
mandatory: a browser can never be served secret tier (ADR-075), and the whole
point is that changing a key changes every client's behaviour on its next config
read. **No secret may ever enter this key set.**

FAIL-SAFE IS THE POINT. Unreachable provider, malformed payload, out-of-range
value → the compiled-in ADR-010 defaults. Never "sample everything out": a
telemetry system that goes silent when its config server hiccups is worse than
useless. A caller whose config read *failed* passes ``None`` and gets exactly
those defaults — which is why this function has no error channel.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any, Final

from .sampling import CanonicalLevel, parse_level

#: boolean — kill switch. ``False`` disables ALL telemetry emission.
KEY_ENABLED: Final = "observabilityEnabled"
#: number 0.0–1.0 — session-scoped browser log sampling ratio.
KEY_BROWSER_LOG_SAMPLING_RATIO: Final = "observabilityBrowserLogSamplingRatio"
#: string — minimum log level (TRACE|DEBUG|INFO|WARN|ERROR|FATAL).
KEY_MINIMUM_LOG_LEVEL: Final = "observabilityMinimumLogLevel"
#: number 0.0–1.0 — head-based trace sampling ratio.
KEY_TRACE_SAMPLING_RATIO: Final = "observabilityTraceSamplingRatio"


@dataclass(frozen=True)
class TelemetrySettings:
    """Resolved telemetry settings."""

    #: Kill switch. When false nothing is emitted, errors included.
    enabled: bool = True
    #: Session-scoped browser log sampling ratio. Applied ONCE per session (or
    #: inherited from the trace where one exists) — never per line.
    browser_log_sampling_ratio: float = 1.0
    #: Minimum level to emit.
    minimum_log_level: CanonicalLevel = CanonicalLevel.INFO
    #: Head-based trace sampling ratio. ADR-010 default: TraceIdRatioBased(0.1).
    trace_sampling_ratio: float = 0.1


#: Compiled-in ADR-010 defaults. Every failure path lands here.
DEFAULT_TELEMETRY_SETTINGS: Final = TelemetrySettings()


def _coerce_ratio(raw: Any, fallback: float) -> float:
    """Finite number, or decimal numeric string → clamped into ``[0, 1]``.

    Public config often round-trips values as strings, so strings are accepted;
    an operator who writes 1.5 means "all". Anything else (missing, NaN,
    Infinity, boolean, object, unparseable string) → the compiled-in default,
    never 0.

    The asymmetry is deliberate: a *malformed* value falls back, a *valid but
    out-of-range* value is clamped. -1 clamps to 0 (telemetry off) because that
    is an explicit operator value, and 0 is settable anyway.

    ``bool`` is excluded explicitly — in Python ``isinstance(True, int)`` is
    True, so ``observabilityEnabled``-style booleans would otherwise coerce to
    a ratio of 1.0 instead of falling back.
    """
    if isinstance(raw, bool):
        return fallback
    if isinstance(raw, (int, float)):
        n = float(raw)
    elif isinstance(raw, str) and raw.strip():
        try:
            n = float(raw.strip())
        except ValueError:
            return fallback
    else:
        return fallback
    if not math.isfinite(n):
        return fallback
    return min(1.0, max(0.0, n))


def _coerce_bool(raw: Any, fallback: bool) -> bool:
    if isinstance(raw, bool):
        return raw
    if isinstance(raw, str):
        s = raw.strip().lower()
        if s == "true":
            return True
        if s == "false":
            return False
    return fallback


def _coerce_level(raw: Any, fallback: CanonicalLevel) -> CanonicalLevel:
    # ``parse_level`` (not ``normalize_level``) on purpose: normalize maps
    # unknown spellings to INFO, which is right for an incoming log line but
    # wrong here — a typo'd config value must fall back to the default, not
    # silently reset the floor.
    return (parse_level(raw) if isinstance(raw, str) else None) or fallback


def resolve_telemetry_settings(raw: Any) -> TelemetrySettings:
    """Turn a raw config payload into settings.

    Total function — never raises, always returns a usable object.
    Unknown/extra keys are ignored.
    """
    d = DEFAULT_TELEMETRY_SETTINGS
    if not isinstance(raw, dict):
        return d
    return TelemetrySettings(
        enabled=_coerce_bool(raw.get(KEY_ENABLED), d.enabled),
        browser_log_sampling_ratio=_coerce_ratio(raw.get(KEY_BROWSER_LOG_SAMPLING_RATIO), d.browser_log_sampling_ratio),
        minimum_log_level=_coerce_level(raw.get(KEY_MINIMUM_LOG_LEVEL), d.minimum_log_level),
        trace_sampling_ratio=_coerce_ratio(raw.get(KEY_TRACE_SAMPLING_RATIO), d.trace_sampling_ratio),
    )
