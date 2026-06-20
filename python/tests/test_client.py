import pytest

from smooai_observability import scope as scope_mod
from smooai_observability.client import Client, ClientOptions
from smooai_observability.scope import Scope
from smooai_observability.types import ObservabilityEvent, User


@pytest.fixture(autouse=True)
def _reset_client():
    Client.init(ClientOptions(dsn="https://example.test/webhook", environment="test", release="abc123"))
    Client.register_transport(None)
    Client.register_capture_handler(None)
    scope_mod._current_scope.set(Scope())  # fresh root scope per test
    yield
    Client.register_transport(None)
    Client.register_capture_handler(None)
    Client._options = None  # reset singleton between tests


def _capture_sink():
    captured: list[ObservabilityEvent] = []
    Client.register_transport(lambda batch: captured.extend(batch))
    return captured


def test_capture_exception_builds_event():
    captured = _capture_sink()
    try:
        raise ValueError("boom")
    except ValueError as err:
        eid = Client.capture_exception(err, tags={"area": "x"})
    assert eid is not None
    assert len(captured) == 1
    ev = captured[0]
    assert ev.level == "error"
    assert ev.exception[0].type == "ValueError"
    assert ev.exception[0].value == "boom"
    assert ev.tags["area"] == "x"
    assert ev.environment == "test"
    assert ev.release == "abc123"
    assert ev.sdk.runtime == "python"


def test_capture_exception_chains_cause():
    captured = _capture_sink()
    try:
        try:
            raise ValueError("root")
        except ValueError as root:
            raise RuntimeError("wrapper") from root
    except RuntimeError as err:
        Client.capture_exception(err)
    ev = captured[0]
    assert ev.exception[0].type == "RuntimeError"
    assert ev.exception[0].cause.type == "ValueError"
    assert ev.exception[0].cause.value == "root"


def test_capture_message_scrubs_pii():
    captured = _capture_sink()
    Client.capture_message("login with Bearer abc.def-ghi token", "warning")
    ev = captured[0]
    assert "Bearer [redacted]" in ev.message
    assert ev.level == "warning"


def test_dual_path_handler_and_transport():
    captured = _capture_sink()
    handler_calls: list[ObservabilityEvent] = []
    Client.register_capture_handler(lambda ev, raw: handler_calls.append(ev))
    Client.capture_message("hi")
    assert len(handler_calls) == 1
    assert len(captured) == 1
    assert handler_calls[0].event_id == captured[0].event_id


def test_before_send_can_drop():
    captured = _capture_sink()
    Client._options.before_send = lambda ev: None  # drop everything
    eid = Client.capture_message("dropped")
    assert eid is not None  # id still returned
    assert captured == []


def test_throwing_transport_is_swallowed():
    def boom(_batch):
        raise RuntimeError("transport down")

    Client.register_transport(boom)
    # Must not raise.
    assert Client.capture_message("x") is not None


def test_uninitialized_client_returns_none():
    Client._options = None
    assert Client.capture_message("x") is None


def test_set_user_flows_to_event():
    captured = _capture_sink()
    Client.set_user(User(id="u9", org_id="o9"))
    Client.capture_message("hello")
    ev = captured[0]
    assert ev.user.id == "u9"
    assert ev.user.org_id == "o9"
