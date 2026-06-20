"""Auth-injecting OTLP/HTTP exporters — port of
``packages/core/src/otel/auth-injecting-exporter.ts``.

The TS port hand-rolls the SpanExporter contract because OTel JS snapshots
headers at construction. The Python OTLP/HTTP exporters don't have that exact
bug, but they DO snapshot headers into a ``requests.Session`` at construction —
so a token minted once goes stale the same way. Rather than reimplement the
protobuf encoding + retry loop, we subclass the upstream exporters and override
the single ``_export`` hook to stamp a **fresh** ``Authorization`` header onto
the session immediately before each POST. The token comes from the
``TokenProvider`` (cache-hit unless expiring), so there's no snapshot and no
expiry drift.

On a 401 we invalidate the cached token so the next export re-mints — mirroring
the TS exporter's one-shot 401 retry (the upstream retry loop treats 401 as
non-retryable, so the re-mint takes effect on the following batch).
"""

from __future__ import annotations

from opentelemetry.exporter.otlp.proto.http.metric_exporter import OTLPMetricExporter
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter

from ..auth.token_provider import TokenProvider


def _inject(session_headers, token_provider: TokenProvider) -> None:
    try:
        token = token_provider.get_access_token()
        session_headers["Authorization"] = f"Bearer {token}"
    except Exception:
        # Never block export on a mint failure — the request will 401 and the
        # token gets invalidated for the next attempt.
        pass


class AuthInjectingTraceExporter(OTLPSpanExporter):
    """OTLP/HTTP span exporter that injects a fresh Bearer per export."""

    def __init__(
        self,
        *,
        endpoint: str,
        token_provider: TokenProvider,
        headers: dict[str, str] | None = None,
        timeout: float | None = None,
    ) -> None:
        super().__init__(endpoint=endpoint, headers=headers, timeout=timeout)
        self._token_provider = token_provider

    def _export(self, serialized_data, timeout_sec=None):  # type: ignore[override]
        _inject(self._session.headers, self._token_provider)
        resp = super()._export(serialized_data, timeout_sec)
        if getattr(resp, "status_code", None) == 401:
            self._token_provider.invalidate()
        return resp


class AuthInjectingMetricExporter(OTLPMetricExporter):
    """OTLP/HTTP metric exporter that injects a fresh Bearer per export."""

    def __init__(
        self,
        *,
        endpoint: str,
        token_provider: TokenProvider,
        headers: dict[str, str] | None = None,
        timeout: float | None = None,
    ) -> None:
        super().__init__(endpoint=endpoint, headers=headers, timeout=timeout)
        self._token_provider = token_provider

    def _export(self, serialized_data, timeout_sec=None):  # type: ignore[override]
        _inject(self._session.headers, self._token_provider)
        resp = super()._export(serialized_data, timeout_sec)
        if getattr(resp, "status_code", None) == 401:
            self._token_provider.invalidate()
        return resp
