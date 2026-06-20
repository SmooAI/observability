"""FastAPI / Starlette middleware — the Python analog of
``packages/core/src/node/middleware.ts`` (which is Hono-shaped).

For each request it:
  1. Pushes a fresh scope (contextvars — async-safe, the AsyncLocalStorage the
     TS middleware comment flagged as a future need).
  2. Resolves user / org / session from the request (default reads
     ``request.state.auth``) and attaches it to the scope.
  3. Records a ``request`` context (method, path, allowlisted headers).
  4. On a thrown error, calls ``capture_exception`` BEFORE re-raising so the
     framework's own error handling still renders the response.

Pure-ASGI (no ``BaseHTTPMiddleware``) so it composes cleanly and doesn't break
streaming responses. ``starlette`` is an OPTIONAL dependency (the ``fastapi``
extra) — importing this module without it raises a clear ImportError, but the
rest of the SDK works fine.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from ..client import Client
from ..scope import with_scope
from ..types import User

try:  # Starlette types are only needed for the Request convenience wrapper.
    from starlette.requests import Request

    _HAS_STARLETTE = True
except ImportError:  # pragma: no cover - import guard
    Request = Any  # type: ignore[assignment,misc]
    _HAS_STARLETTE = False

DEFAULT_HEADER_ALLOWLIST = (
    "user-agent",
    "referer",
    "x-request-id",
    "x-trace-id",
    "x-correlation-id",
)

UserResolver = Callable[[Any], User | None]


def _default_resolve_user(request: Any) -> User | None:
    """Read ``request.state.auth`` (the @smooai/auth-shaped output) if present."""
    state = getattr(request, "state", None)
    auth = getattr(state, "auth", None) if state is not None else None
    if not auth:
        return None
    get = auth.get if isinstance(auth, dict) else lambda k: getattr(auth, k, None)
    user = User(
        id=get("userId") or get("user_id"),
        org_id=get("orgId") or get("org_id"),
        session_id=get("sessionId") or get("session_id"),
    )
    return None if user.is_empty() else user


class ObservabilityMiddleware:
    """Pure-ASGI observability middleware."""

    def __init__(
        self,
        app: Any,
        *,
        resolve_user: UserResolver | None = None,
        request_header_allowlist: tuple[str, ...] | None = None,
    ) -> None:
        if not _HAS_STARLETTE:
            raise ImportError(
                "smooai_observability.integrations.fastapi requires starlette (install the 'fastapi' extra: pip install smooai-observability[fastapi])"
            )
        self.app = app
        self._resolve_user = resolve_user or _default_resolve_user
        self._allowlist = tuple(h.lower() for h in (request_header_allowlist or DEFAULT_HEADER_ALLOWLIST))

    async def __call__(self, scope: dict, receive: Callable, send: Callable) -> None:
        if scope.get("type") != "http" or not Client.is_initialized():
            await self.app(scope, receive, send)
            return

        with with_scope() as obs_scope:
            try:
                request = Request(scope, receive=receive)
                user = self._resolve_user(request)
                if user is not None:
                    obs_scope.set_user(user)

                headers: dict[str, str] = {}
                for name in self._allowlist:
                    value = request.headers.get(name)
                    if value:
                        headers[name] = value
                obs_scope.set_context(
                    "request",
                    {
                        "method": scope.get("method", ""),
                        "path": scope.get("path", ""),
                        "headers": headers,
                    },
                )
            except Exception:
                pass  # scope hydration must not break the request

            try:
                await self.app(scope, receive, send)
            except Exception as err:
                try:
                    Client.capture_exception(err, tags={"source": "fastapi.middleware"})
                except Exception:
                    pass
                raise
