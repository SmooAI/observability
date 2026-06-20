"""SmooAI Observability — Python SDK.

Port of the TypeScript reference SDK at
``~/dev/smooai/observability/packages/core/src/``. Error capture + breadcrumbs +
scoped context (webhook transport, SMOODEV-1148 dual-path), plus OpenTelemetry
traces + metrics export, GenAI semantic conventions, and an M2M token provider.

Quick start (env-driven bootstrap)::

    from smooai_observability import bootstrap_observability, capture_exception

    bootstrap_observability()  # reads SMOOAI_OBSERVABILITY_* env vars

    try:
        risky()
    except Exception as err:
        capture_exception(err, tags={"area": "ingest"})

Submodules: ``smooai_observability.metrics``, ``.otel``, ``.bootstrap``,
``.integrations.fastapi`` — mirroring the TS subpaths.
"""

from __future__ import annotations

from .bootstrap import (
    BootstrapEnv,
    BootstrapResult,
    bootstrap_observability,
)
from .client import Client, ClientOptions
from .gen_ai_attributes import (
    GenAIAttributes,
    record_gen_ai_message,
    set_gen_ai_attributes,
)
from .scope import Scope, get_current_scope, run_with_scope, with_scope
from .types import (
    Breadcrumb,
    ExceptionInfo,
    IngestPayload,
    Level,
    ObservabilityEvent,
    RequestInfo,
    Sdk,
    StackFrame,
    User,
)

__version__ = "0.1.0"


# --- module-level convenience API (mirror TS top-level exports) --------------


def capture_exception(error: BaseException, tags=None):  # noqa: ANN001
    """Capture an exception. Returns the event id (or None if uninitialized)."""
    return Client.capture_exception(error, tags=tags)


def capture_message(message: str, level: Level = "info"):
    """Capture a message at the given level. Returns the event id."""
    return Client.capture_message(message, level=level)


def set_user(user):  # noqa: ANN001
    Client.set_user(user)


def set_tag(key: str, value: str) -> None:
    Client.set_tag(key, value)


def set_context(key: str, ctx) -> None:  # noqa: ANN001
    Client.set_context(key, ctx)


def add_breadcrumb(category: str, message=None, data=None, level: Level = "info") -> None:  # noqa: ANN001
    Client.add_breadcrumb(category, message, data, level)


__all__ = [
    "__version__",
    # client
    "Client",
    "ClientOptions",
    # capture API
    "capture_exception",
    "capture_message",
    "set_user",
    "set_tag",
    "set_context",
    "add_breadcrumb",
    # scope
    "Scope",
    "get_current_scope",
    "with_scope",
    "run_with_scope",
    # bootstrap
    "bootstrap_observability",
    "BootstrapEnv",
    "BootstrapResult",
    # gen-ai
    "GenAIAttributes",
    "set_gen_ai_attributes",
    "record_gen_ai_message",
    # types
    "ObservabilityEvent",
    "ExceptionInfo",
    "StackFrame",
    "Breadcrumb",
    "RequestInfo",
    "User",
    "Sdk",
    "Level",
    "IngestPayload",
]
