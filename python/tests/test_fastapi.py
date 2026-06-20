import pytest

starlette = pytest.importorskip("starlette")

from starlette.applications import Starlette  # noqa: E402
from starlette.responses import PlainTextResponse  # noqa: E402
from starlette.routing import Route  # noqa: E402
from starlette.testclient import TestClient  # noqa: E402

from smooai_observability import scope as scope_mod  # noqa: E402
from smooai_observability.client import Client, ClientOptions  # noqa: E402
from smooai_observability.integrations.fastapi import ObservabilityMiddleware  # noqa: E402
from smooai_observability.scope import Scope  # noqa: E402
from smooai_observability.types import ObservabilityEvent  # noqa: E402


@pytest.fixture(autouse=True)
def _client():
    Client.init(ClientOptions(environment="test"))
    Client.register_transport(None)
    Client.register_capture_handler(None)
    scope_mod._current_scope.set(Scope())  # fresh root scope (no cross-test bleed)
    yield
    Client.register_transport(None)
    Client.register_capture_handler(None)
    Client._options = None


class _AuthMiddleware:
    """Stand-in for the upstream auth middleware that populates
    ``request.state.auth`` BEFORE observability resolves the user — the correct
    production ordering (auth runs outside observability)."""

    def __init__(self, app, auth):
        self.app = app
        self.auth = auth

    async def __call__(self, scope, receive, send):
        if scope["type"] == "http":
            scope.setdefault("state", {})["auth"] = self.auth
        await self.app(scope, receive, send)


def _app():
    captured: list[ObservabilityEvent] = []
    Client.register_transport(lambda batch: captured.extend(batch))

    async def ok(request):
        return PlainTextResponse("ok")

    async def boom(request):
        raise ValueError("handler failed")

    app = Starlette(routes=[Route("/ok", ok), Route("/boom", boom)])
    # Observability inside, auth outside — auth runs first.
    app.add_middleware(ObservabilityMiddleware)
    app.add_middleware(_AuthMiddleware, auth={"userId": "u2", "orgId": "o2"})
    return app, captured


def test_successful_request_passes_through():
    app, captured = _app()
    client = TestClient(app)
    resp = client.get("/ok")
    assert resp.status_code == 200
    assert resp.text == "ok"
    assert captured == []  # no error captured


def test_error_is_captured_with_user_context():
    app, captured = _app()
    client = TestClient(app, raise_server_exceptions=False)
    resp = client.get("/boom", headers={"user-agent": "pytest"})
    assert resp.status_code == 500
    assert len(captured) == 1
    ev = captured[0]
    assert ev.exception[0].type == "ValueError"
    assert ev.user.id == "u2"
    assert ev.user.org_id == "o2"
    assert ev.tags["source"] == "fastapi.middleware"
    # request context recorded with allowlisted header
    assert ev.contexts["request"]["method"] == "GET"
    assert ev.contexts["request"]["path"] == "/boom"
    assert ev.contexts["request"]["headers"]["user-agent"] == "pytest"
