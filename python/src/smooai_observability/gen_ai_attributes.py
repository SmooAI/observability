"""OTel GenAI Semantic Conventions — attribute helpers.

Port of ``packages/core/src/gen-ai-attributes.ts``. Sets the standardized
``gen_ai.*`` attributes on an OTel span so any OTel backend interprets LLM /
agent telemetry with one shared vocabulary.

Spec: https://opentelemetry.io/docs/specs/semconv/gen-ai/
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from opentelemetry.trace import Span

GenAIOperationName = Literal["chat", "text_completion", "embeddings", "tool", "agent", "rerank"]


@dataclass
class GenAIAttributes:
    """Full set of GenAI semantic-convention attributes. All optional — set what
    you know. Keys mirror the spec exactly so backends parse without mapping."""

    system: str | None = None
    operation_name: GenAIOperationName | None = None
    request_model: str | None = None
    response_model: str | None = None
    response_id: str | None = None
    temperature: float | None = None
    top_p: float | None = None
    top_k: float | None = None
    max_tokens: int | None = None
    seed: int | None = None
    usage_input_tokens: int | None = None
    usage_output_tokens: int | None = None
    usage_cached_tokens: int | None = None
    usage_cost_usd: float | None = None
    tool_names: list[str] | None = None
    truncated: bool | None = None
    finish_reason: str | None = None
    end_user_id: str | None = None
    conversation_id: str | None = None


def set_gen_ai_attributes(span: Span, attrs: GenAIAttributes) -> None:
    """Apply a ``GenAIAttributes`` onto a span. Skips unset fields so partial
    calls are idempotent."""
    if attrs.system is not None:
        span.set_attribute("gen_ai.system", attrs.system)
    if attrs.operation_name is not None:
        span.set_attribute("gen_ai.operation.name", attrs.operation_name)
    if attrs.request_model is not None:
        span.set_attribute("gen_ai.request.model", attrs.request_model)
    if attrs.response_model is not None:
        span.set_attribute("gen_ai.response.model", attrs.response_model)
    if attrs.response_id is not None:
        span.set_attribute("gen_ai.response.id", attrs.response_id)
    if attrs.temperature is not None:
        span.set_attribute("gen_ai.request.temperature", attrs.temperature)
    if attrs.top_p is not None:
        span.set_attribute("gen_ai.request.top_p", attrs.top_p)
    if attrs.top_k is not None:
        span.set_attribute("gen_ai.request.top_k", attrs.top_k)
    if attrs.max_tokens is not None:
        span.set_attribute("gen_ai.request.max_tokens", attrs.max_tokens)
    if attrs.seed is not None:
        span.set_attribute("gen_ai.request.seed", attrs.seed)
    if attrs.usage_input_tokens is not None:
        span.set_attribute("gen_ai.usage.input_tokens", attrs.usage_input_tokens)
    if attrs.usage_output_tokens is not None:
        span.set_attribute("gen_ai.usage.output_tokens", attrs.usage_output_tokens)
    if attrs.usage_cached_tokens is not None:
        span.set_attribute("gen_ai.usage.cached_tokens", attrs.usage_cached_tokens)
    if attrs.usage_cost_usd is not None:
        span.set_attribute("gen_ai.usage.cost_usd", attrs.usage_cost_usd)
    if attrs.tool_names:
        span.set_attribute("gen_ai.tool.names", attrs.tool_names)
    if attrs.truncated is not None:
        span.set_attribute("gen_ai.response.truncated", attrs.truncated)
    if attrs.finish_reason is not None:
        span.set_attribute("gen_ai.response.finish_reason", attrs.finish_reason)
    if attrs.end_user_id is not None:
        span.set_attribute("gen_ai.end_user.id", attrs.end_user_id)
    if attrs.conversation_id is not None:
        span.set_attribute("gen_ai.conversation.id", attrs.conversation_id)


def record_gen_ai_message(
    span: Span,
    role: Literal["user", "assistant", "system", "tool"],
    content: str,
    *,
    tool_call_id: str | None = None,
    tool_name: str | None = None,
) -> None:
    """Emit a ``gen_ai.{role}.message`` span event with the message content.
    Use sparingly — these are size-heavy."""
    attributes: dict[str, object] = {"gen_ai.message.content": content}
    if tool_call_id is not None:
        attributes["gen_ai.tool_call.id"] = tool_call_id
    if tool_name is not None:
        attributes["gen_ai.tool.name"] = tool_name
    span.add_event(f"gen_ai.{role}.message", attributes)
