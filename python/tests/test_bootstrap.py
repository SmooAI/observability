from smooai_observability.bootstrap import (
    BootstrapEnv,
    bootstrap_observability,
    reset_bootstrap_for_tests,
)
from smooai_observability.client import Client
from smooai_observability.otel import reset_otel_capture_for_tests, reset_otel_sdk_for_tests


def _reset():
    reset_bootstrap_for_tests()
    reset_otel_sdk_for_tests()
    reset_otel_capture_for_tests()
    Client._options = None


def test_disabled_short_circuits():
    _reset()
    try:
        result = bootstrap_observability(BootstrapEnv(disabled=True))
        assert result.installed is False
        assert result.otel is None
    finally:
        _reset()


def test_idempotent():
    _reset()
    try:
        r1 = bootstrap_observability(BootstrapEnv(service_name="svc"), fetch_token=False)
        r2 = bootstrap_observability(BootstrapEnv(service_name="other"), fetch_token=False)
        assert r1 is r2
    finally:
        _reset()


def test_installs_and_inits_client_with_static_token():
    _reset()
    try:
        result = bootstrap_observability(
            BootstrapEnv(
                endpoint="https://api.test",
                token="pre-minted-jwt",
                service_name="svc",
                environment="staging",
            ),
            fetch_token=False,
        )
        assert result.installed is True
        assert Client.is_initialized()
        opts = Client.get_options()
        assert opts.environment == "staging"
    finally:
        _reset()


def test_never_raises_on_bad_config():
    _reset()
    try:
        # No auth, no endpoint — must still return a result, not raise.
        result = bootstrap_observability(BootstrapEnv(), fetch_token=False)
        assert result.installed is True
    finally:
        _reset()
