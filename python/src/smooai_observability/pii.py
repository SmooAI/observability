"""PII scrubbing — applied to message strings, breadcrumb messages, and headers
before transport. Port of ``rust/observability/src/pii.rs``; the semantics are
identical across the five SDKs.

Two classes, handled differently on purpose:

- **Credentials** (``Bearer …``, ``password=``, ``token``/``api_key``/``secret=``,
  ``sk-…``) are **dropped**. A hash of a live token is still a token oracle, and
  there is no correlation value in a secret.
- **Personal identifiers** (email, phone, street address) are **hashed**, not
  dropped: ``a@b.com`` → ``[email:9f2a41c8]``. That keeps the one question worth
  asking — "are these two spans the same person?" — answerable while storing
  nothing reversible.

The hash is **HMAC-SHA256, keyed**, not a bare digest: emails and phone numbers
are a small enumerable space that a rainbow table reverses in seconds. The org
id is mixed into the HMAC message, so identical PII hashes **differently in
different orgs**.

**The key and the org salt are load-bearing and must not rotate casually.**
Rotating either silently breaks correlation with every previously stored hash.
Supply the key once at startup via ``SMOOAI_OBSERVABILITY_PII_HASH_KEY`` (read
by :mod:`smooai_observability.bootstrap`) or :func:`set_pii_hash_key`. **With no
key configured, personal identifiers are fully redacted** (``[email:redacted]``)
rather than hashed under a guessable one — fail safe, never fail open.

Pattern matching never catches everything. It fails safe, which is the right
default for data we persist from end-user conversations; tenants can extend in
``before_send``.
"""

from __future__ import annotations

import hmac
import re
import threading
from enum import StrEnum

# --- credentials: matched FIRST, dropped entirely --------------------------
#
# A personal identifier sitting inside a secret (``token=a@b.com``) is dropped
# with the secret rather than surviving as a hash.

_BEARER_RE = re.compile(r"Bearer\s+[A-Za-z0-9._-]+", re.IGNORECASE)
_PASSWORD_RE = re.compile(
    r"""\b(?:password|passwd|pwd)["']?\s*[:=]\s*["']?[^"'&\s]+""",
    re.IGNORECASE,
)
# The TS source's replacement string resolves to a no-op (`$&`). Python keeps
# the leading `key=`/`key:` and redacts only the value — the intended effect,
# and what the Rust reference does.
_TOKEN_RE = re.compile(
    r"""\b(?P<key>(?:token|api[-_]?key|apikey|secret)["']?\s*[:=]\s*["']?)[^"'&\s]+""",
    re.IGNORECASE,
)
_SK_RE = re.compile(r"\bsk-[A-Za-z0-9]{20,}")

_SENSITIVE_HEADERS = frozenset({"authorization", "cookie", "set-cookie", "x-api-key", "x-auth-token"})

# Hex characters kept from the HMAC. Long enough that collisions are rare across
# an org's traces, short enough to read in a span attribute.
_HASH_HEX_LEN = 8

_NON_DIGIT_RE = re.compile(r"[^0-9]")


class PiiKind(StrEnum):
    """The class of personal identifier a match represents.

    Drives both the visible prefix in the output token and the normalization
    applied before hashing, so ``(415) 555-0142`` and ``415-555-0142``
    correlate.
    """

    EMAIL = "email"
    PHONE = "phone"
    ADDRESS = "address"

    @property
    def label(self) -> str:
        """The prefix that stays visible in the scrubbed output."""
        return self.value

    def normalize(self, raw: str) -> str:
        if self is PiiKind.EMAIL:
            return raw.strip().lower()
        if self is PiiKind.PHONE:
            # Digits only: formatting must not fork the hash.
            return _NON_DIGIT_RE.sub("", raw)
        # Case-fold and collapse runs of whitespace.
        return " ".join(raw.split()).lower()


# --- personal identifiers: hashed, not dropped -----------------------------
#
# These all run after the credential patterns above.
_PERSONAL_PATTERNS: list[tuple[PiiKind, re.Pattern[str]]] = [
    (
        PiiKind.EMAIL,
        re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+\b", re.IGNORECASE),
    ),
    # Phone: optional country code / area code, then the 3-4 local pair. A
    # separator is REQUIRED so bare digit runs (ids, timestamps, amounts) don't
    # get eaten.
    (
        PiiKind.PHONE,
        re.compile(r"(?:\+\d{1,3}[ .-]?)?(?:\(\d{3}\)[ .-]?|\b\d{3}[ .-])?\b\d{3}[ .-]\d{4}\b"),
    ),
    # US-style street address: house number, 1-3 words, a street suffix.
    (
        PiiKind.ADDRESS,
        re.compile(
            r"\b\d{1,6}\s+(?:[A-Za-z0-9.'-]+\s+){0,3}"
            r"(?:street|st|avenue|ave|road|rd|boulevard|blvd|lane|ln|drive|dr|court|ct|way|"
            r"terrace|ter|place|pl|circle|cir|highway|hwy|parkway|pkwy|square|sq)\b\.?",
            re.IGNORECASE,
        ),
    ),
]

_key_lock = threading.Lock()
_pii_hash_key: bytes | None = None


def set_pii_hash_key(key: bytes | str) -> bool:
    """Install the process-wide HMAC key used to hash personal identifiers.

    Idempotent and set-once: returns ``False`` if a key was already installed
    (the existing key is kept — a mid-process rotation would silently fork every
    correlation) or if ``key`` is empty. The bootstrap calls this from
    ``SMOOAI_OBSERVABILITY_PII_HASH_KEY``.
    """
    global _pii_hash_key
    raw = key.encode() if isinstance(key, str) else key
    if not raw:
        return False
    with _key_lock:
        if _pii_hash_key is not None:
            return False
        _pii_hash_key = raw
        return True


def _current_key() -> bytes | None:
    return _pii_hash_key


def pii_token(kind: PiiKind, raw: str, org_id: str) -> str:
    """Hash one known-personal value into its scrubbed token.

    Produces exactly the token :func:`scrub_string_for_org` would have written,
    which is how a UI search box finds stored hashes: hash the typed term with
    the same org and match.

    Returns ``[<kind>:redacted]`` when no key is installed.
    """
    return _token_with_key(kind, raw, org_id, _current_key())


def _token_with_key(kind: PiiKind, raw: str, org_id: str, key: bytes | None) -> str:
    if not key:
        return f"[{kind.label}:redacted]"
    # org_id in the HMAC message IS the per-org salt. The kind is in there too so
    # a phone and an address that normalize alike can't collide.
    msg = b"\x00".join((org_id.encode(), kind.label.encode(), kind.normalize(raw).encode()))
    digest = hmac.new(key, msg, "sha256").hexdigest()[:_HASH_HEX_LEN]
    return f"[{kind.label}:{digest}]"


def scrub_string(value: str) -> str:
    """Scrub a free-form string with no org context.

    Credentials dropped, personal identifiers hashed under the empty org salt.
    Prefer :func:`scrub_string_for_org` wherever an org id is in hand, so hashes
    can't be correlated across tenants.
    """
    return _scrub_with_key(value, "", _current_key())


def scrub_string_for_org(value: str, org_id: str) -> str:
    """Scrub a free-form string, salting personal-identifier hashes with ``org_id``."""
    return _scrub_with_key(value, org_id, _current_key())


def _scrub_with_key(value: str, org_id: str, key: bytes | None) -> str:
    out = _BEARER_RE.sub("Bearer [redacted]", value)
    out = _PASSWORD_RE.sub("password=[redacted]", out)
    out = _TOKEN_RE.sub(lambda m: f"{m.group('key')}[redacted]", out)
    out = _SK_RE.sub("sk-[redacted]", out)
    for kind, pattern in _PERSONAL_PATTERNS:
        out = pattern.sub(lambda m, k=kind: _token_with_key(k, m.group(0), org_id, key), out)
    return out


def scrub_headers(
    headers: dict[str, str] | None,
) -> dict[str, str] | None:
    """Redact sensitive headers wholesale; scrub remaining header values."""
    return scrub_headers_for_org(headers, "")


def scrub_headers_for_org(
    headers: dict[str, str] | None,
    org_id: str,
) -> dict[str, str] | None:
    """:func:`scrub_headers` with an org salt for the personal-identifier hashes."""
    if not headers:
        return headers
    out: dict[str, str] = {}
    for k, v in headers.items():
        out[k] = "[redacted]" if k.lower() in _SENSITIVE_HEADERS else scrub_string_for_org(v, org_id)
    return out
