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


def test_never_raises_on_bad_config(monkeypatch, capsys):
    _reset()
    try:
        # setup_otel_sdk falls back to these, so "no endpoint" has to mean no
        # endpoint from ANY source or the exporting assertion below would be
        # environment-dependent.
        for key in (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
            "SMOOAI_OBSERVABILITY_ENDPOINT",
        ):
            monkeypatch.delenv(key, raising=False)

        # No auth, no endpoint — must still return a result, not raise.
        result = bootstrap_observability(BootstrapEnv(), fetch_token=False)
        assert result.installed is True  # bootstrap ran…
        # …but it is NOT exporting, and the result now says so. This assertion
        # is the whole point: `installed` alone used to be the only signal, and
        # it reads as "everything is fine" while nothing leaves the process.
        assert result.exporting is False
        stderr = capsys.readouterr().err
        assert "NO OTLP ENDPOINT CONFIGURED" in stderr
        assert "SMOOAI_OBSERVABILITY_DISABLED=true" in stderr
    finally:
        _reset()


def test_exporting_true_when_endpoint_configured(capsys):
    """The inverse of the no-endpoint case. Without both halves asserted, a
    regression that hard-codes either value passes."""
    _reset()
    try:
        result = bootstrap_observability(
            BootstrapEnv(endpoint="https://collector.example.test", token="pre-minted", service_name="svc"),
            fetch_token=False,
        )
        assert result.installed is True
        assert result.exporting is True
        assert "NO OTLP ENDPOINT CONFIGURED" not in capsys.readouterr().err
    finally:
        _reset()
