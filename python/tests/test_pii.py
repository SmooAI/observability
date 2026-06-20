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
