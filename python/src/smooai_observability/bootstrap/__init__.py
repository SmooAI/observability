"""One-call bootstrap — port of ``packages/core/src/bootstrap/index.ts``.

Reads config from the same ``SMOOAI_OBSERVABILITY_*`` env vars as the TS SDK, so
the same code path serves customer compute and Smoo's own services with the only
difference being where the env vars come from.

Unlike the TS side-effect import (``import '@smooai/observability/bootstrap'``),
Python callers run ``bootstrap_observability()`` explicitly near process start
(it's idempotent). It NEVER raises — missing config, mint failures, and OTel
init errors are logged to stderr and the SDK falls back to a no-op.

## Required env vars
  SMOOAI_OBSERVABILITY_ENDPOINT   — base ingest URL (e.g. "https://api.smoo.ai").
                                    ``/v1/traces`` + ``/v1/metrics`` + ``/v1/logs`` are appended.

## Auth (pick ONE; pre-minted token wins if both present)
  SMOOAI_OBSERVABILITY_TOKEN          — pre-minted Bearer JWT (not refreshed).
  --- or ---
  SMOOAI_OBSERVABILITY_AUTH_URL       — OAuth /token base (e.g. "https://auth.smoo.ai").
  SMOOAI_OBSERVABILITY_CLIENT_ID      — M2M client id.
  SMOOAI_OBSERVABILITY_CLIENT_SECRET  — M2M client secret.

## Optional env vars
  SMOOAI_OBSERVABILITY_SERVICE_NAME   — default "smoo-service".
  SMOOAI_OBSERVABILITY_ENVIRONMENT    — default STAGE / ENVIRONMENT / NODE_ENV.
  SMOOAI_OBSERVABILITY_RELEASE        — default GIT_SHA / "dev".
  SMOOAI_OBSERVABILITY_DSN            — webhook DSN for the Errors dashboard.
  SMOOAI_OBSERVABILITY_DISABLED       — "1"/"true" to skip bootstrap entirely.
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass

from ..auth.token_provider import TokenProvider, TokenProviderError
from ..client import Client, ClientOptions
from ..transport import Transport


@dataclass
class BootstrapEnv:
    endpoint: str | None = None
    traces_endpoint: str | None = None
    metrics_endpoint: str | None = None
    logs_endpoint: str | None = None
    token: str | None = None
    auth_url: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    dsn: str | None = None
    service_name: str = "smoo-service"
    environment: str | None = None
    release: str | None = None
    disabled: bool = False


@dataclass
class BootstrapResult:
    installed: bool
    otel: object = None  # OtelSdkHandle | None
    transport: Transport | None = None


_bootstrapped: BootstrapResult | None = None


def _warn(message: str) -> None:
    try:
        sys.stderr.write(f"[@smooai/observability/bootstrap] {message}\n")
    except Exception:
        pass


def _truthy(value: str | None) -> bool:
    if not value:
        return False
    return value == "1" or value.lower() == "true"


def _strip_trailing_slash(url: str) -> str:
    return url[:-1] if url.endswith("/") else url


def bootstrap_observability(
    overrides: BootstrapEnv | None = None,
    *,
    fetch_token: bool = True,
) -> BootstrapResult:
    """Run the bootstrap (idempotent). Returns a result describing what was
    installed. Never raises."""
    global _bootstrapped
    if _bootstrapped is not None:
        return _bootstrapped

    o = overrides or BootstrapEnv()
    env = BootstrapEnv(
        endpoint=o.endpoint or os.environ.get("SMOOAI_OBSERVABILITY_ENDPOINT"),
        traces_endpoint=o.traces_endpoint or os.environ.get("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
        metrics_endpoint=o.metrics_endpoint or os.environ.get("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"),
        logs_endpoint=o.logs_endpoint or os.environ.get("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"),
        token=o.token or os.environ.get("SMOOAI_OBSERVABILITY_TOKEN"),
        auth_url=o.auth_url or os.environ.get("SMOOAI_OBSERVABILITY_AUTH_URL"),
        client_id=o.client_id or os.environ.get("SMOOAI_OBSERVABILITY_CLIENT_ID"),
        client_secret=o.client_secret or os.environ.get("SMOOAI_OBSERVABILITY_CLIENT_SECRET"),
        dsn=o.dsn or os.environ.get("SMOOAI_OBSERVABILITY_DSN"),
        service_name=o.service_name if o.service_name != "smoo-service" else os.environ.get("SMOOAI_OBSERVABILITY_SERVICE_NAME", "smoo-service"),
        environment=o.environment
        or os.environ.get("SMOOAI_OBSERVABILITY_ENVIRONMENT")
        or os.environ.get("STAGE")
        or os.environ.get("ENVIRONMENT")
        or os.environ.get("NODE_ENV"),
        release=o.release or os.environ.get("SMOOAI_OBSERVABILITY_RELEASE") or os.environ.get("GIT_SHA") or "dev",
        disabled=o.disabled or _truthy(os.environ.get("SMOOAI_OBSERVABILITY_DISABLED")),
    )

    if env.disabled:
        _bootstrapped = BootstrapResult(installed=False)
        return _bootstrapped

    token_provider: TokenProvider | None = None
    static_headers: dict[str, str] = {}
    try:
        if env.token:
            static_headers["Authorization"] = f"Bearer {env.token}"
        elif env.auth_url and env.client_id and env.client_secret:
            token_provider = TokenProvider(
                auth_url=env.auth_url,
                client_id=env.client_id,
                client_secret=env.client_secret,
            )
            if fetch_token:
                # Warm the cache so the first export doesn't pay the round-trip.
                try:
                    token_provider.get_access_token()
                except (TokenProviderError, Exception) as mint_err:
                    _warn(f"initial token mint failed; exports will retry: {mint_err}")
        else:
            _warn("no auth configured (set SMOOAI_OBSERVABILITY_TOKEN or _AUTH_URL/_CLIENT_ID/_CLIENT_SECRET); OTLP exports will be unauthenticated")

        traces_endpoint = env.traces_endpoint or (f"{_strip_trailing_slash(env.endpoint)}/v1/traces" if env.endpoint else None)
        metrics_endpoint = env.metrics_endpoint or (f"{_strip_trailing_slash(env.endpoint)}/v1/metrics" if env.endpoint else None)
        logs_endpoint = env.logs_endpoint or (f"{_strip_trailing_slash(env.endpoint)}/v1/logs" if env.endpoint else None)

        otel_handle = _maybe_setup_otel(
            service_name=env.service_name,
            environment=env.environment,
            release=env.release,
            traces_endpoint=traces_endpoint,
            metrics_endpoint=metrics_endpoint,
            logs_endpoint=logs_endpoint,
            static_headers=static_headers,
            token_provider=token_provider,
        )

        # Webhook transport for the Errors dashboard (TS dual-path parity). Only
        # wired when a DSN is present — OTel-native capture alone is fine
        # otherwise.
        transport: Transport | None = None
        Client.init(
            ClientOptions(
                dsn=env.dsn,
                environment=env.environment,
                release=env.release,
            )
        )
        if env.dsn:
            transport = Transport(env.dsn)

            def _send(batch: list) -> None:
                for event in batch:
                    transport.enqueue(event)

            Client.register_transport(_send)

        _register_otel_capture()

        _bootstrapped = BootstrapResult(installed=True, otel=otel_handle, transport=transport)
    except Exception as err:
        _warn(f"SDK init failed: {err}")
        _bootstrapped = BootstrapResult(installed=False)
    return _bootstrapped


def _maybe_setup_otel(**kwargs: object) -> object:
    try:
        from ..otel import SetupOtelOptions, setup_otel_sdk
    except Exception as err:  # pragma: no cover
        _warn(f"otel import failed; tracing/metrics disabled: {err}")
        return None
    return setup_otel_sdk(
        SetupOtelOptions(
            service_name=str(kwargs["service_name"]),
            environment=kwargs.get("environment"),  # type: ignore[arg-type]
            release=kwargs.get("release"),  # type: ignore[arg-type]
            otlp_endpoint=kwargs.get("traces_endpoint"),  # type: ignore[arg-type]
            otlp_metrics_endpoint=kwargs.get("metrics_endpoint"),  # type: ignore[arg-type]
            otlp_logs_endpoint=kwargs.get("logs_endpoint"),  # type: ignore[arg-type]
            otlp_headers=dict(kwargs.get("static_headers") or {}),  # type: ignore[arg-type]
            token_provider=kwargs.get("token_provider"),  # type: ignore[arg-type]
        )
    )


def _register_otel_capture() -> None:
    try:
        from ..otel import register_otel_capture

        register_otel_capture()
    except Exception as err:  # pragma: no cover
        _warn(f"otel capture registration failed: {err}")


def reset_bootstrap_for_tests() -> None:
    """Test seam — NOT exported from the package entry."""
    global _bootstrapped
    if _bootstrapped is not None and _bootstrapped.transport is not None:
        try:
            _bootstrapped.transport.shutdown()
        except Exception:
            pass
    _bootstrapped = None
