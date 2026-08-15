import pytest

from smooai_observability import pii


def test_scrub_bearer_token():
    assert pii.scrub_string("Authorization: Bearer abc.def-ghi_123") == "Authorization: Bearer [redacted]"


def test_scrub_password():
    assert "[redacted]" in pii.scrub_string("password=hunter2")
    assert "hunter2" not in pii.scrub_string("password=hunter2")
    assert "hunter2" not in pii.scrub_string('pwd: "hunter2"')


def test_scrub_token_keeps_key_redacts_value():
    out = pii.scrub_string("api_key=sk_live_supersecretvalue")
    assert "supersecretvalue" not in out
    assert "[redacted]" in out
    out2 = pii.scrub_string("token: abc123def")
    assert "abc123def" not in out2


def test_scrub_openai_style_key():
    out = pii.scrub_string("key is sk-ABCDEFGHIJKLMNOPQRSTUVWX")
    assert out == "key is sk-[redacted]"


def test_scrub_headers_sensitive_wholesale():
    headers = {
        "Authorization": "Bearer xyz",
        "Cookie": "session=abc",
        "User-Agent": "test",
        "X-Api-Key": "secret",
    }
    out = pii.scrub_headers(headers)
    assert out["Authorization"] == "[redacted]"
    assert out["Cookie"] == "[redacted]"
    assert out["X-Api-Key"] == "[redacted]"
    assert out["User-Agent"] == "test"


def test_scrub_headers_none_passthrough():
    assert pii.scrub_headers(None) is None
    assert pii.scrub_headers({}) == {}


def test_scrub_headers_scrubs_nonsensitive_values():
    out = pii.scrub_headers({"X-Note": "Bearer leaked.token"})
    assert out["X-Note"] == "Bearer [redacted]"


# ---- PII hashing parity with rust/observability/src/pii.rs -----------------
#
# These drive the private _scrub_with_key / _token_with_key rather than
# installing the process-wide key: set_pii_hash_key is set-once and pytest runs
# the suite in one process, so a global write would make it order-dependent.

KEY = b"test-hmac-key-not-a-real-secret"
OTHER_KEY = b"a-different-test-hmac-key"


def scrub(value: str, org: str = "org-1") -> str:
    return pii._scrub_with_key(value, org, KEY)


def test_credentials_are_dropped_not_hashed():
    # A live token must never become a correlatable handle — hashing one still
    # yields an oracle you can test candidate tokens against.
    out = scrub("Bearer abc.def-ghi_123 password=hunter2 sk-ABCDEFGHIJKLMNOPQRSTUVWX")
    assert "Bearer [redacted]" in out
    assert "password=[redacted]" in out
    assert "sk-[redacted]" in out
    for shape in ("[token:", "[credential:", "[email:", "[phone:"):
        assert shape not in out, out
    # And a personal identifier hiding inside a secret goes with it.
    out2 = scrub("token=a@b.com")
    assert "a@b.com" not in out2
    assert "[email:" not in out2


def test_hashes_emails_keeping_the_type_prefix():
    out = scrub("contact me at Alice@Example.com please")
    assert "alice@example.com" not in out.lower()
    assert out.startswith("contact me at [email:")
    assert out.endswith("] please")


def test_hashes_phone_numbers():
    for raw in ("555-0142", "(415) 555-0142", "+1 415-555-0142"):
        out = scrub(f"call {raw} today")
        assert "[phone:" in out, out
        assert "0142" not in out, out


def test_hashes_street_addresses():
    out = scrub("ship to 1600 Pennsylvania Ave, Washington")
    assert "[address:" in out, out
    assert "Pennsylvania" not in out, out


def test_same_value_same_org_is_stable():
    assert scrub("a@b.com") == scrub("a@b.com")
    # …and correlation survives formatting differences in phones.
    assert scrub("(415) 555-0142") == scrub("415-555-0142")


def test_same_value_different_org_hashes_differently():
    a = scrub("a@b.com", "org-1")
    b = scrub("a@b.com", "org-2")
    assert a != b, f"per-org salt missing: {a} == {b}"
    assert a.startswith("[email:") and b.startswith("[email:")


def test_different_key_hashes_differently():
    a = pii._scrub_with_key("a@b.com", "org-1", KEY)
    b = pii._scrub_with_key("a@b.com", "org-1", OTHER_KEY)
    assert a != b, f"hash is not keyed: {a} == {b}"


def test_no_key_redacts_rather_than_hashing():
    # Fail safe: an unkeyed digest of an email is rainbow-tabled instantly.
    out = pii._scrub_with_key("a@b.com and 555-0142", "org-1", None)
    assert "[email:redacted]" in out, out
    assert "[phone:redacted]" in out, out
    assert "a@b.com" not in out
    assert "0142" not in out


def test_hash_is_short_hex():
    token = pii._token_with_key(pii.PiiKind.EMAIL, "a@b.com", "org-1", KEY)
    digest = token.removeprefix("[email:").removesuffix("]")
    assert len(digest) == pii._HASH_HEX_LEN, token
    assert all(c in "0123456789abcdef" for c in digest), token


def test_pii_token_matches_what_scrubbing_wrote():
    # The searchability contract: hashing a typed query term must produce
    # exactly the token stored in the span.
    scrubbed = pii._scrub_with_key("mail a@b.com now", "org-7", KEY)
    term = pii._token_with_key(pii.PiiKind.EMAIL, "A@B.com ", "org-7", KEY)
    assert term in scrubbed, f"{scrubbed} vs {term}"


def test_ordinary_numbers_are_not_mistaken_for_phones():
    # The phone pattern requires a separator before the last 4 digits; ids,
    # versions, dates and amounts must survive intact.
    for value in (
        "took 1234 ms",
        "version 1.2.3",
        "2026-08-15T12:00:00Z",
        "total 1234.5678",
        "trace 0af7651916cd43dd8448eb211c80319c",
    ):
        assert scrub(value) == value, f"false positive on {value}"


def test_set_pii_hash_key_is_set_once_and_rejects_empty():
    assert pii.set_pii_hash_key(b"") is False
    saved = pii._pii_hash_key
    pii._pii_hash_key = None
    try:
        assert pii.set_pii_hash_key("first-key") is True
        assert pii.set_pii_hash_key("second-key") is False, "rotation should be refused"
        assert pii._pii_hash_key == b"first-key"
    finally:
        pii._pii_hash_key = saved


def test_scrub_headers_for_org_never_leaks_raw_email():
    out = pii.scrub_headers_for_org({"X-Note": "reply to a@b.com"}, "org-1")
    assert "a@b.com" not in out["X-Note"]
    assert "[email:" in out["X-Note"]


def test_bootstrap_reads_pii_hash_key_from_env(monkeypatch):
    from smooai_observability import bootstrap as bootstrap_mod

    monkeypatch.setenv("SMOOAI_OBSERVABILITY_PII_HASH_KEY", "env-supplied-key")
    monkeypatch.setenv("SMOOAI_OBSERVABILITY_DISABLED", "1")
    monkeypatch.setattr(bootstrap_mod, "_bootstrapped", None)
    saved = pii._pii_hash_key
    pii._pii_hash_key = None
    try:
        bootstrap_mod.bootstrap_observability()
        assert pii._pii_hash_key == b"env-supplied-key"
    finally:
        pii._pii_hash_key = saved
        bootstrap_mod._bootstrapped = None


@pytest.mark.parametrize(
    ("kind", "raw", "org", "expected"),
    [
        (pii.PiiKind.EMAIL, "a@b.com", "org-1", "[email:02ea437f]"),
        (pii.PiiKind.EMAIL, "A@B.COM ", "org-1", "[email:02ea437f]"),
        (pii.PiiKind.EMAIL, "a@b.com", "org-2", "[email:fd96f7dc]"),
        (pii.PiiKind.EMAIL, "a@b.com", "", "[email:453b154f]"),
        (pii.PiiKind.PHONE, "(415) 555-0142", "org-1", "[phone:415a9aea]"),
        (pii.PiiKind.PHONE, "415-555-0142", "org-1", "[phone:415a9aea]"),
        (pii.PiiKind.ADDRESS, "1600  Pennsylvania   Ave", "org-1", "[address:c5351f4a]"),
    ],
)
def test_cross_sdk_parity_vectors(kind, raw, org, expected):
    """Pins the exact bytes every SDK must produce.

    Computed independently and asserted verbatim in all five SDKs. If any SDK's
    message framing, normalization or truncation drifts, exactly one of these
    breaks.
    """
    assert pii._token_with_key(kind, raw, org, KEY) == expected
