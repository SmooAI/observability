"""Wire-format tests — the serialized dict MUST match the TS camelCase shape."""

from __future__ import annotations

from smooai_observability.types import (
    Breadcrumb,
    ExceptionInfo,
    IngestPayload,
    ObservabilityEvent,
    RequestInfo,
    Sdk,
    StackFrame,
    User,
)


def test_stack_frame_camelcase_and_omit_none():
    f = StackFrame(module="app.py", function="handler", lineno=10, colno=3, in_app=True)
    assert f.to_wire() == {
        "module": "app.py",
        "function": "handler",
        "lineno": 10,
        "colno": 3,
        "inApp": True,
    }
    # None fields omitted, in_app -> inApp
    minimal = StackFrame(module="x.py")
    assert minimal.to_wire() == {"module": "x.py"}


def test_exception_nested_stacktrace_and_cause():
    inner = ExceptionInfo(type="ValueError", value="bad", stacktrace=[StackFrame(module="a.py")])
    outer = ExceptionInfo(type="RuntimeError", value="wrap", stacktrace=[], cause=inner)
    wire = outer.to_wire()
    assert wire["type"] == "RuntimeError"
    assert wire["stacktrace"] == {"frames": []}
    assert wire["cause"]["type"] == "ValueError"
    assert wire["cause"]["stacktrace"]["frames"][0]["module"] == "a.py"


def test_breadcrumb_wire():
    b = Breadcrumb(timestamp=123, category="custom", level="warning", message="hi", data={"k": 1})
    assert b.to_wire() == {
        "timestamp": 123,
        "category": "custom",
        "level": "warning",
        "message": "hi",
        "data": {"k": 1},
    }


def test_request_info_query_string_key():
    r = RequestInfo(url="/x", method="GET", headers={"a": "b"}, query_string="q=1")
    wire = r.to_wire()
    assert wire["queryString"] == "q=1"
    assert "query_string" not in wire


def test_event_user_omitted_when_empty():
    ev = ObservabilityEvent(
        event_id="id",
        timestamp=1,
        level="error",
        sdk=Sdk("@smooai/observability", "0.1.0", "python"),
        user=User(),  # empty
    )
    wire = ev.to_wire()
    assert "user" not in wire
    assert wire["eventId"] == "id"
    assert wire["sdk"] == {"name": "@smooai/observability", "version": "0.1.0", "runtime": "python"}


def test_ingest_payload_shape():
    ev = ObservabilityEvent(
        event_id="id",
        timestamp=1,
        level="info",
        message="m",
        sdk=Sdk("@smooai/observability", "0.1.0", "python"),
    )
    payload = IngestPayload(events=[ev])
    wire = payload.to_wire()
    assert wire["type"] == "error"
    assert isinstance(wire["events"], list)
    assert wire["events"][0]["message"] == "m"
