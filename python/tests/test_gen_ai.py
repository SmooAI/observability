from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter

from smooai_observability.gen_ai_attributes import (
    GenAIAttributes,
    record_gen_ai_message,
    set_gen_ai_attributes,
)


def _provider():
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    return provider, exporter


def test_set_gen_ai_attributes_maps_spec_keys():
    provider, exporter = _provider()
    tracer = provider.get_tracer("test")
    with tracer.start_as_current_span("llm.call") as span:
        set_gen_ai_attributes(
            span,
            GenAIAttributes(
                system="anthropic",
                operation_name="chat",
                request_model="claude-opus-4-8",
                usage_input_tokens=10,
                usage_output_tokens=20,
                tool_names=["search"],
                finish_reason="stop",
            ),
        )
    spans = exporter.get_finished_spans()
    attrs = spans[0].attributes
    assert attrs["gen_ai.system"] == "anthropic"
    assert attrs["gen_ai.operation.name"] == "chat"
    assert attrs["gen_ai.request.model"] == "claude-opus-4-8"
    assert attrs["gen_ai.usage.input_tokens"] == 10
    assert attrs["gen_ai.usage.output_tokens"] == 20
    assert tuple(attrs["gen_ai.tool.names"]) == ("search",)
    assert attrs["gen_ai.response.finish_reason"] == "stop"
    # Unset fields must not appear.
    assert "gen_ai.request.temperature" not in attrs


def test_record_gen_ai_message_event():
    provider, exporter = _provider()
    tracer = provider.get_tracer("test")
    with tracer.start_as_current_span("llm.call") as span:
        record_gen_ai_message(span, "user", "hello", tool_name="search", tool_call_id="tc1")
    span = exporter.get_finished_spans()[0]
    event = span.events[0]
    assert event.name == "gen_ai.user.message"
    assert event.attributes["gen_ai.message.content"] == "hello"
    assert event.attributes["gen_ai.tool.name"] == "search"
    assert event.attributes["gen_ai.tool_call.id"] == "tc1"


# ---- GenAI cross-SDK divergences, closed ----------------------------------


def test_tool_names_is_a_string_array():
    """Pins the shape the Rust SDK used to get wrong (comma-joined string).

    A backend filtering by tool cannot do it against a joined string, and a tool
    name containing a comma silently became two tools.
    """
    provider, exporter = _provider()
    with provider.get_tracer("test").start_as_current_span("llm.call") as span:
        set_gen_ai_attributes(span, GenAIAttributes(tool_names=["search", "calc"]))
    attrs = exporter.get_finished_spans()[0].attributes
    assert tuple(attrs["gen_ai.tool.names"]) == ("search", "calc")
    assert not isinstance(attrs["gen_ai.tool.names"], str)


def test_recorded_message_content_is_scrubbed():
    """Prompts and tool arguments are the most PII-dense payload this SDK touches.

    Only the TS SDK used to scrub them.
    """
    provider, exporter = _provider()
    with provider.get_tracer("test").start_as_current_span("llm.call") as span:
        record_gen_ai_message(span, "user", "mail a@b.com, Authorization: Bearer abc.def-ghi")
    event = exporter.get_finished_spans()[0].events[0]
    content = event.attributes["gen_ai.message.content"]
    assert "a@b.com" not in content
    assert "Bearer [redacted]" in content
    # No key installed in this test process, so the email is redacted rather
    # than hashed — the fail-safe, not a missed match.
    assert "[email:" in content
