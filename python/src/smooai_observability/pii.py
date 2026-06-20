"""PII scrubbing — applied to message strings, breadcrumb messages, and headers
before transport. Stays opinionated and minimal; tenants can extend in
``before_send``.

Direct port of ``packages/core/src/pii.ts``. The regex patterns mirror the TS
source. One deliberate deviation: the TS ``token|api_key|secret`` pattern ships
a latent no-op replacement (``'$&'.replace(/=.*/, ...)`` evaluates to the
literal ``$&`` at module load, so the match is replaced with itself). A PII
scrubber that doesn't redact secrets is worse than useless, so the Python port
actually redacts the value while keeping the key — the clearly-intended
behavior. See the inline note on ``_TOKEN_RE``.
"""

from __future__ import annotations

import re

# Mirrors the TS PII_PATTERNS. Python's `re` uses `(?i)` / re.IGNORECASE for
# the `i` flag and re.sub iterates globally by default (no `g` flag needed).
_BEARER_RE = re.compile(r"Bearer\s+[A-Za-z0-9._-]+", re.IGNORECASE)
_PASSWORD_RE = re.compile(
    r"""\b(?:password|passwd|pwd)["']?\s*[:=]\s*["']?[^"'&\s]+""",
    re.IGNORECASE,
)
# TS source: the replacement string resolves to a no-op (`$&`). Python keeps
# the leading `key=`/`key:` and redacts only the value — the intended effect.
_TOKEN_RE = re.compile(
    r"""\b(?P<key>(?:token|api[-_]?key|apikey|secret)["']?\s*[:=]\s*["']?)[^"'&\s]+""",
    re.IGNORECASE,
)
_SK_RE = re.compile(r"\bsk-[A-Za-z0-9]{20,}")

_SENSITIVE_HEADERS = frozenset({"authorization", "cookie", "set-cookie", "x-api-key", "x-auth-token"})


def scrub_string(value: str) -> str:
    """Redact known secret shapes from a free-form string."""
    out = _BEARER_RE.sub("Bearer [redacted]", value)
    out = _PASSWORD_RE.sub("password=[redacted]", out)
    out = _TOKEN_RE.sub(lambda m: f"{m.group('key')}[redacted]", out)
    out = _SK_RE.sub("sk-[redacted]", out)
    return out


def scrub_headers(
    headers: dict[str, str] | None,
) -> dict[str, str] | None:
    """Redact sensitive headers wholesale; scrub remaining header values."""
    if not headers:
        return headers
    out: dict[str, str] = {}
    for k, v in headers.items():
        out[k] = "[redacted]" if k.lower() in _SENSITIVE_HEADERS else scrub_string(v)
    return out
