"""OTel — trace + metric SDK bootstrap and auth-injecting exporters."""

from .capture import register_otel_capture, reset_otel_capture_for_tests
from .setup_otel_sdk import (
    OtelSdkHandle,
    SetupOtelOptions,
    reset_otel_sdk_for_tests,
    setup_otel_sdk,
)

__all__ = [
    "OtelSdkHandle",
    "SetupOtelOptions",
    "setup_otel_sdk",
    "reset_otel_sdk_for_tests",
    "register_otel_capture",
    "reset_otel_capture_for_tests",
]
