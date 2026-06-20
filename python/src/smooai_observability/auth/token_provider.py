"""OAuth2 client_credentials token provider — port of
``packages/core/src/auth/token-provider.ts``.

Authenticates against ``api.smoo.ai`` exactly the way every other smooai client
does. The token is consulted at *request* time by the OTLP exporter (no header
snapshot, no staleness): cached in memory until ``refresh_window_sec`` before
expiry, then re-minted. Concurrent callers during a refresh share one in-flight
request.

The OTel Python OTLP/HTTP exporter exports on a sync ``requests``/``http``
session, so this provider is **synchronous** + thread-safe (a lock serializes
the refresh, the cache is a plain attribute read on the hot path). Server
contract::

    POST {auth_url}/token
    Content-Type: application/x-www-form-urlencoded

    grant_type=client_credentials
    provider=client_credentials
    client_id=<uuid>
    client_secret=sk_...
"""

from __future__ import annotations

import threading
import time
from collections.abc import Callable
from dataclasses import dataclass

import httpx


class TokenProviderError(RuntimeError):
    """Raised when the OAuth token exchange fails."""


@dataclass
class _CachedToken:
    access_token: str
    expires_at: int  # unix epoch seconds


class TokenProvider:
    def __init__(
        self,
        *,
        auth_url: str,
        client_id: str,
        client_secret: str,
        refresh_window_sec: int = 60,
        client: httpx.Client | None = None,
        now: Callable[[], float] | None = None,
    ) -> None:
        if not auth_url:
            raise ValueError("@smooai/observability: TokenProvider requires auth_url")
        if not client_id:
            raise ValueError("@smooai/observability: TokenProvider requires client_id")
        if not client_secret:
            raise ValueError("@smooai/observability: TokenProvider requires client_secret")
        self._auth_url = auth_url.rstrip("/")
        self._client_id = client_id
        self._client_secret = client_secret
        self._refresh_window_sec = refresh_window_sec
        self._owns_client = client is None
        self._client = client or httpx.Client(timeout=10.0)
        self._now = now or time.time
        self._cached: _CachedToken | None = None
        self._lock = threading.Lock()

    def get_access_token(self) -> str:
        """Return a valid token, refreshing if missing / expiring."""
        if not self._should_refresh():
            assert self._cached is not None
            return self._cached.access_token
        with self._lock:
            # Re-check under the lock — another thread may have refreshed.
            if not self._should_refresh():
                assert self._cached is not None
                return self._cached.access_token
            return self._refresh()

    def invalidate(self) -> None:
        """Drop the cached token. Call after a 401 so the next attempt re-mints."""
        with self._lock:
            self._cached = None

    def _should_refresh(self) -> bool:
        if self._cached is None:
            return True
        now_sec = int(self._now())
        return now_sec >= self._cached.expires_at - self._refresh_window_sec

    def _refresh(self) -> str:
        resp = self._client.post(
            f"{self._auth_url}/token",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            data={
                "grant_type": "client_credentials",
                "provider": "client_credentials",
                "client_id": self._client_id,
                "client_secret": self._client_secret,
            },
        )
        if resp.status_code >= 400:
            body = resp.text[:300] if resp.text else "<unreadable>"
            raise TokenProviderError(f"@smooai/observability: OAuth token exchange failed: HTTP {resp.status_code} {body}")
        body = resp.json()
        access_token = body.get("access_token")
        if not access_token:
            raise TokenProviderError("@smooai/observability: OAuth token endpoint returned no access_token")
        expires_in = body.get("expires_in")
        if not isinstance(expires_in, int | float):
            expires_in = 3600
        now_sec = int(self._now())
        self._cached = _CachedToken(access_token=access_token, expires_at=now_sec + int(expires_in))
        return access_token

    def close(self) -> None:
        if self._owns_client:
            try:
                self._client.close()
            except Exception:
                pass
