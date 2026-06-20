import httpx
import pytest

from smooai_observability.auth import TokenProvider, TokenProviderError


def _client(handler):
    return httpx.Client(transport=httpx.MockTransport(handler))


def test_mints_and_caches():
    calls = {"n": 0}

    def handler(request):
        calls["n"] += 1
        body = dict(p.split("=") for p in request.content.decode().split("&"))
        assert body["grant_type"] == "client_credentials"
        assert body["provider"] == "client_credentials"
        return httpx.Response(200, json={"access_token": "tok-1", "expires_in": 3600})

    tp = TokenProvider(
        auth_url="https://auth.test/",
        client_id="cid",
        client_secret="sk_secret",
        client=_client(handler),
    )
    assert tp.get_access_token() == "tok-1"
    assert tp.get_access_token() == "tok-1"  # cached
    assert calls["n"] == 1


def test_refresh_before_expiry():
    now = {"t": 1_000_000.0}
    tokens = iter(["tok-a", "tok-b"])

    def handler(request):
        return httpx.Response(200, json={"access_token": next(tokens), "expires_in": 100})

    tp = TokenProvider(
        auth_url="https://auth.test",
        client_id="cid",
        client_secret="sk",
        refresh_window_sec=60,
        client=_client(handler),
        now=lambda: now["t"],
    )
    assert tp.get_access_token() == "tok-a"
    # Advance to within the refresh window (100 - 60 = 40s of validity left at +50s).
    now["t"] += 50
    assert tp.get_access_token() == "tok-b"


def test_invalidate_forces_remint():
    tokens = iter(["tok-1", "tok-2"])

    def handler(request):
        return httpx.Response(200, json={"access_token": next(tokens), "expires_in": 3600})

    tp = TokenProvider(auth_url="https://auth.test", client_id="c", client_secret="s", client=_client(handler))
    assert tp.get_access_token() == "tok-1"
    tp.invalidate()
    assert tp.get_access_token() == "tok-2"


def test_raises_on_http_error():
    def handler(request):
        return httpx.Response(401, text="bad creds")

    tp = TokenProvider(auth_url="https://auth.test", client_id="c", client_secret="s", client=_client(handler))
    with pytest.raises(TokenProviderError):
        tp.get_access_token()


def test_requires_fields():
    with pytest.raises(ValueError):
        TokenProvider(auth_url="", client_id="c", client_secret="s")
    with pytest.raises(ValueError):
        TokenProvider(auth_url="u", client_id="", client_secret="s")
