"""Tests for the LangChain/LangGraph callback handler.

These mock LangChain's callback invocations directly — we drive the handler's
``on_*`` methods with the payload shapes LangChain produces, so no live LLM or
``langchain`` runtime is required. ``langchain-core`` is installed in the dev
group only so the handler can subclass ``BaseCallbackHandler``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any
from uuid import uuid4

import pytest
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import StatusCode

from smooai_observability.integrations.langchain import SmooAICallbackHandler

# --- minimal stand-ins for langchain_core result objects ---------------------


@dataclass
class _Message:
    response_metadata: dict[str, Any] = field(default_factory=dict)


@dataclass
class _Generation:
    generation_info: dict[str, Any] | None = None
    message: _Message | None = None


@dataclass
class _LLMResult:
    """Mirrors ``langchain_core.outputs.LLMResult`` enough for the handler."""

    generations: list[list[_Generation]] = field(default_factory=list)
    llm_output: dict[str, Any] | None = None


@pytest.fixture
def exporter_and_handler():
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    tracer = provider.get_tracer("test")
    handler = SmooAICallbackHandler(tracer=tracer)
    return exporter, handler


def _attrs(exporter: InMemorySpanExporter, name: str) -> dict[str, Any]:
    spans = [s for s in exporter.get_finished_spans() if s.name == name]
    assert spans, f"no finished span named {name!r}; got {[s.name for s in exporter.get_finished_spans()]}"
    return dict(spans[-1].attributes)


def test_llm_start_end_sets_gen_ai_attributes(exporter_and_handler):
    exporter, handler = exporter_and_handler
    run_id = uuid4()
    serialized = {"id": ["langchain", "chat_models", "anthropic", "ChatAnthropic"], "name": "ChatAnthropic"}

    handler.on_llm_start(
        serialized,
        ["hello"],
        run_id=run_id,
        invocation_params={"model": "claude-opus-4-8", "temperature": 0.5, "max_tokens": 1024},
    )
    result = _LLMResult(
        generations=[[_Generation(generation_info={"finish_reason": "stop"})]],
        llm_output={
            "model_name": "claude-opus-4-8",
            "id": "msg_123",
            "token_usage": {"prompt_tokens": 12, "completion_tokens": 34},
        },
    )
    handler.on_llm_end(result, run_id=run_id)

    attrs = _attrs(exporter, "chat claude-opus-4-8")
    assert attrs["gen_ai.system"] == "anthropic"
    assert attrs["gen_ai.operation.name"] == "chat"
    assert attrs["gen_ai.request.model"] == "claude-opus-4-8"
    assert attrs["gen_ai.request.temperature"] == 0.5
    assert attrs["gen_ai.request.max_tokens"] == 1024
    assert attrs["gen_ai.response.model"] == "claude-opus-4-8"
    assert attrs["gen_ai.response.id"] == "msg_123"
    assert attrs["gen_ai.usage.input_tokens"] == 12
    assert attrs["gen_ai.usage.output_tokens"] == 34
    assert attrs["gen_ai.response.finish_reason"] == "stop"


def test_chat_model_start_counts_messages_and_infers_openai(exporter_and_handler):
    exporter, handler = exporter_and_handler
    run_id = uuid4()
    serialized = {"id": ["langchain", "chat_models", "openai", "ChatOpenAI"], "name": "ChatOpenAI"}

    handler.on_chat_model_start(
        serialized,
        [[_Message(), _Message()]],
        run_id=run_id,
        invocation_params={"model_name": "gpt-5.4", "top_p": 0.9},
    )
    result = _LLMResult(
        generations=[[_Generation(message=_Message(response_metadata={"finish_reason": "length"}))]],
        llm_output={
            "model_name": "gpt-5.4",
            "token_usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "prompt_tokens_details": {"cached_tokens": 40},
            },
        },
    )
    handler.on_llm_end(result, run_id=run_id)

    attrs = _attrs(exporter, "chat gpt-5.4")
    assert attrs["gen_ai.system"] == "openai"
    assert attrs["gen_ai.request.prompt_count"] == 2
    assert attrs["gen_ai.request.top_p"] == 0.9
    assert attrs["gen_ai.usage.input_tokens"] == 100
    assert attrs["gen_ai.usage.output_tokens"] == 50
    assert attrs["gen_ai.usage.cached_tokens"] == 40
    assert attrs["gen_ai.response.finish_reason"] == "length"


def test_usage_metadata_shape(exporter_and_handler):
    exporter, handler = exporter_and_handler
    run_id = uuid4()
    handler.on_llm_start({"name": "ChatAnthropic"}, ["x"], run_id=run_id, invocation_params={"model": "claude-haiku-4-5"})
    result = _LLMResult(
        llm_output={
            "usage_metadata": {
                "input_tokens": 7,
                "output_tokens": 9,
                "input_token_details": {"cache_read": 3},
            }
        },
    )
    handler.on_llm_end(result, run_id=run_id)
    attrs = _attrs(exporter, "chat claude-haiku-4-5")
    assert attrs["gen_ai.usage.input_tokens"] == 7
    assert attrs["gen_ai.usage.output_tokens"] == 9
    assert attrs["gen_ai.usage.cached_tokens"] == 3


def test_llm_error_records_exception(exporter_and_handler):
    exporter, handler = exporter_and_handler
    run_id = uuid4()
    handler.on_llm_start({"name": "ChatAnthropic"}, ["x"], run_id=run_id, invocation_params={"model": "claude-opus-4-8"})
    handler.on_llm_error(RuntimeError("boom"), run_id=run_id)

    span = [s for s in exporter.get_finished_spans() if s.name == "chat claude-opus-4-8"][-1]
    assert span.status.status_code == StatusCode.ERROR
    assert any(e.name == "exception" for e in span.events)


def test_tool_span(exporter_and_handler):
    exporter, handler = exporter_and_handler
    run_id = uuid4()
    handler.on_tool_start({"name": "search"}, "query text", run_id=run_id)
    handler.on_tool_end("result text", run_id=run_id)

    attrs = _attrs(exporter, "tool search")
    assert attrs["gen_ai.operation.name"] == "tool"
    assert attrs["gen_ai.tool.name"] == "search"
    assert tuple(attrs["gen_ai.tool.names"]) == ("search",)
    # content not captured by default
    assert "gen_ai.tool.input" not in attrs
    assert "gen_ai.tool.output" not in attrs


def test_tool_content_capture(exporter_and_handler):
    exporter, _ = exporter_and_handler
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    handler = SmooAICallbackHandler(tracer=provider.get_tracer("t"), capture_content=True)
    run_id = uuid4()
    handler.on_tool_start({"name": "calc"}, "1+1", run_id=run_id)
    handler.on_tool_end("2", run_id=run_id)
    attrs = _attrs(exporter, "tool calc")
    assert attrs["gen_ai.tool.input"] == "1+1"
    assert attrs["gen_ai.tool.output"] == "2"


def test_tool_error(exporter_and_handler):
    exporter, handler = exporter_and_handler
    run_id = uuid4()
    handler.on_tool_start({"name": "search"}, "q", run_id=run_id)
    handler.on_tool_error(ValueError("bad"), run_id=run_id)
    span = [s for s in exporter.get_finished_spans() if s.name == "tool search"][-1]
    assert span.status.status_code == StatusCode.ERROR


def test_chain_nesting_parents_child_to_chain(exporter_and_handler):
    exporter, handler = exporter_and_handler
    chain_id = uuid4()
    llm_id = uuid4()

    handler.on_chain_start({"name": "agent"}, {"input": "hi"}, run_id=chain_id)
    handler.on_llm_start(
        {"name": "ChatAnthropic"},
        ["hi"],
        run_id=llm_id,
        parent_run_id=chain_id,
        invocation_params={"model": "claude-opus-4-8"},
    )
    handler.on_llm_end(_LLMResult(llm_output={"token_usage": {"prompt_tokens": 1, "completion_tokens": 2}}), run_id=llm_id)
    handler.on_chain_end({"output": "done"}, run_id=chain_id)

    chain_span = [s for s in exporter.get_finished_spans() if s.name == "chain agent"][-1]
    llm_span = [s for s in exporter.get_finished_spans() if s.name == "chat claude-opus-4-8"][-1]

    assert dict(chain_span.attributes)["langchain.chain.name"] == "agent"
    # child LLM span shares the chain's trace and is parented to it
    assert llm_span.context.trace_id == chain_span.context.trace_id
    assert llm_span.parent is not None
    assert llm_span.parent.span_id == chain_span.context.span_id


def test_unknown_run_id_end_is_noop(exporter_and_handler):
    exporter, handler = exporter_and_handler
    # on_llm_end for a run we never started must not raise
    handler.on_llm_end(_LLMResult(), run_id=uuid4())
    assert exporter.get_finished_spans() == ()


def test_system_inference_fallback(exporter_and_handler):
    exporter, handler = exporter_and_handler
    run_id = uuid4()
    handler.on_llm_start({"name": "MysteryModel"}, ["x"], run_id=run_id, invocation_params={"model": "foo"})
    handler.on_llm_end(_LLMResult(), run_id=run_id)
    attrs = _attrs(exporter, "chat foo")
    assert attrs["gen_ai.system"] == "langchain"


def test_import_guard_message():
    # When langchain-core IS installed (dev group), construction succeeds.
    # This documents that the guard path raises a clear ImportError otherwise.
    handler = SmooAICallbackHandler()
    assert handler is not None
