"""LangChain / LangGraph integration — a ``BaseCallbackHandler`` that emits
OTel GenAI spans for LLM / chat / tool / chain runs.

LangChain drives every model, tool, and chain invocation through its callback
system, handing each run a unique ``run_id`` (and ``parent_run_id`` for nesting).
This handler opens an OTel span on each ``*_start`` and closes it on the matching
``*_end`` / ``*_error``, decorating LLM / chat spans with the standardized
``gen_ai.*`` attributes via :mod:`smooai_observability.gen_ai_attributes` so any
OTel backend reads the telemetry with one shared vocabulary.

LangChain callbacks run on whatever thread/executor invoked the chain, and runs
can overlap, so this handler keeps a ``run_id -> (Span, context-token)`` registry
guarded by a lock rather than relying on the implicit current-span context. Spans
are parented explicitly via ``parent_run_id`` so the LangGraph tree is preserved.

``langchain-core`` is an OPTIONAL dependency (the ``langchain`` extra). Importing
this module without it raises a clear ImportError, but the rest of the SDK works
fine.

Usage::

    from smooai_observability.integrations.langchain import SmooAICallbackHandler

    handler = SmooAICallbackHandler()
    agent.invoke({"messages": [...]}, config={"callbacks": [handler]})
"""

from __future__ import annotations

import threading
from typing import Any
from uuid import UUID

from opentelemetry import trace as otel_trace
from opentelemetry.trace import Span, SpanKind, Status, StatusCode

from ..gen_ai_attributes import GenAIAttributes, set_gen_ai_attributes

try:  # langchain-core is only needed to subclass the real base handler.
    from langchain_core.callbacks.base import BaseCallbackHandler

    _HAS_LANGCHAIN = True
except ImportError:  # pragma: no cover - import guard
    BaseCallbackHandler = object  # type: ignore[assignment,misc]
    _HAS_LANGCHAIN = False

# Default OTel system value when the model provider can't be inferred.
DEFAULT_SYSTEM = "langchain"

# Map common LangChain provider / class-name fragments to OTel ``gen_ai.system``
# values (https://opentelemetry.io/docs/specs/semconv/gen-ai/). Lowercased
# substring match against the model class name + serialized provider hints.
_SYSTEM_HINTS: tuple[tuple[str, str], ...] = (
    ("anthropic", "anthropic"),
    ("claude", "anthropic"),
    ("bedrock", "aws.bedrock"),
    ("azureopenai", "azure.ai.openai"),
    ("azure", "azure.ai.openai"),
    ("openai", "openai"),
    ("vertex", "vertex_ai"),
    ("googlegenerativeai", "gcp.gemini"),
    ("gemini", "gcp.gemini"),
    ("google", "gcp.gemini"),
    ("groq", "groq"),
    ("cohere", "cohere"),
    ("mistral", "mistral_ai"),
    ("ollama", "ollama"),
    ("fireworks", "fireworks"),
    ("together", "together_ai"),
)


def _infer_system(serialized: dict[str, Any] | None, kwargs: dict[str, Any]) -> str:
    """Best-effort ``gen_ai.system`` from the serialized model id + invocation
    params. Falls back to ``langchain`` when nothing matches."""
    haystack_parts: list[str] = []
    if serialized:
        cls_id = serialized.get("id")
        if isinstance(cls_id, list):
            haystack_parts.extend(str(p) for p in cls_id)
        elif cls_id is not None:
            haystack_parts.append(str(cls_id))
        name = serialized.get("name")
        if name:
            haystack_parts.append(str(name))
    params = kwargs.get("invocation_params")
    if isinstance(params, dict):
        provider = params.get("_type") or params.get("provider")
        if provider:
            haystack_parts.append(str(provider))
    haystack = " ".join(haystack_parts).lower()
    for fragment, system in _SYSTEM_HINTS:
        if fragment in haystack:
            return system
    return DEFAULT_SYSTEM


def _request_model(serialized: dict[str, Any] | None, kwargs: dict[str, Any]) -> str | None:
    """Pull the request model name from invocation params or serialized kwargs."""
    params = kwargs.get("invocation_params")
    if isinstance(params, dict):
        model = params.get("model") or params.get("model_name") or params.get("model_id") or params.get("deployment_name")
        if model:
            return str(model)
    if serialized:
        s_kwargs = serialized.get("kwargs")
        if isinstance(s_kwargs, dict):
            model = s_kwargs.get("model") or s_kwargs.get("model_name") or s_kwargs.get("model_id")
            if model:
                return str(model)
    return None


def _request_params(kwargs: dict[str, Any]) -> dict[str, Any]:
    """Extract optional sampling params from invocation_params."""
    params = kwargs.get("invocation_params")
    return params if isinstance(params, dict) else {}


def _as_int(value: Any) -> int | None:
    try:
        if value is None:
            return None
        return int(value)
    except (TypeError, ValueError):
        return None


def _as_float(value: Any) -> float | None:
    try:
        if value is None:
            return None
        return float(value)
    except (TypeError, ValueError):
        return None


def _extract_usage(llm_output: dict[str, Any] | None) -> tuple[int | None, int | None, int | None]:
    """Return ``(input_tokens, output_tokens, cached_tokens)`` from an
    ``LLMResult.llm_output`` dict. Handles the OpenAI ``token_usage`` shape and
    the newer LangChain ``usage_metadata`` / Anthropic ``usage`` shapes."""
    if not isinstance(llm_output, dict):
        return None, None, None

    usage = llm_output.get("token_usage") or llm_output.get("usage") or llm_output.get("usage_metadata")
    if not isinstance(usage, dict):
        return None, None, None

    input_tokens = _as_int(usage.get("prompt_tokens") or usage.get("input_tokens"))
    output_tokens = _as_int(usage.get("completion_tokens") or usage.get("output_tokens"))

    cached: int | None = None
    details = usage.get("input_token_details")
    if isinstance(details, dict):
        cached = _as_int(details.get("cache_read") or details.get("cached_tokens"))
    if cached is None:
        prompt_details = usage.get("prompt_tokens_details")
        if isinstance(prompt_details, dict):
            cached = _as_int(prompt_details.get("cached_tokens"))
    if cached is None:
        cached = _as_int(usage.get("cache_read_input_tokens"))

    return input_tokens, output_tokens, cached


def _extract_finish_reason(response: Any) -> str | None:
    """Pull the finish/stop reason out of an ``LLMResult``."""
    generations = getattr(response, "generations", None)
    if not generations:
        return None
    for batch in generations:
        for gen in batch:
            info = getattr(gen, "generation_info", None)
            if isinstance(info, dict):
                reason = info.get("finish_reason") or info.get("stop_reason")
                if reason:
                    return str(reason)
            message = getattr(gen, "message", None)
            meta = getattr(message, "response_metadata", None)
            if isinstance(meta, dict):
                reason = meta.get("finish_reason") or meta.get("stop_reason")
                if reason:
                    return str(reason)
    return None


def _response_model_and_id(response: Any) -> tuple[str | None, str | None]:
    """Pull response model + id from ``LLMResult.llm_output`` / message metadata."""
    llm_output = getattr(response, "llm_output", None)
    model: str | None = None
    response_id: str | None = None
    if isinstance(llm_output, dict):
        model = llm_output.get("model_name") or llm_output.get("model")
        response_id = llm_output.get("id") or llm_output.get("system_fingerprint")
    if model is None or response_id is None:
        generations = getattr(response, "generations", None) or []
        for batch in generations:
            for gen in batch:
                message = getattr(gen, "message", None)
                meta = getattr(message, "response_metadata", None)
                if isinstance(meta, dict):
                    model = model or meta.get("model_name") or meta.get("model")
                    response_id = response_id or meta.get("id")
    return (str(model) if model else None, str(response_id) if response_id else None)


class SmooAICallbackHandler(BaseCallbackHandler):  # type: ignore[misc]
    """LangChain ``BaseCallbackHandler`` that emits OTel GenAI spans.

    Args:
        tracer: An OTel ``Tracer``. Defaults to the global provider's tracer for
            this SDK, so it works after ``setup_otel_sdk`` / ``bootstrap``.
        capture_content: When ``True``, records prompt/completion text as span
            attributes. Off by default — content is size-heavy and may be PII.
    """

    # LangChain inspects these to decide whether to invoke the handler at all.
    raise_error = False
    run_inline = True

    def __init__(self, *, tracer: Any = None, capture_content: bool = False) -> None:
        if not _HAS_LANGCHAIN:
            raise ImportError(
                "smooai_observability.integrations.langchain requires langchain-core "
                "(install the 'langchain' extra: pip install smooai-observability[langchain])"
            )
        self._tracer = tracer or otel_trace.get_tracer("smooai_observability.langchain")
        self._capture_content = capture_content
        # run_id -> span. Guarded by _lock because LangChain may fire callbacks
        # from worker threads for parallel runs. Spans are parented explicitly
        # via parent_run_id rather than the implicit current-span context, which
        # is unreliable across LangChain's executor threads.
        self._spans: dict[UUID, Span] = {}
        self._lock = threading.Lock()

    # --- span registry helpers ------------------------------------------------

    def _start_span(
        self,
        run_id: UUID,
        parent_run_id: UUID | None,
        name: str,
        kind: SpanKind,
    ) -> Span:
        parent_context = None
        if parent_run_id is not None:
            with self._lock:
                parent = self._spans.get(parent_run_id)
            if parent is not None:
                parent_context = otel_trace.set_span_in_context(parent)
        span = self._tracer.start_span(name, context=parent_context, kind=kind)
        with self._lock:
            self._spans[run_id] = span
        return span

    def _pop_span(self, run_id: UUID) -> Span | None:
        with self._lock:
            return self._spans.pop(run_id, None)

    # --- LLM / chat model -----------------------------------------------------

    def on_llm_start(
        self,
        serialized: dict[str, Any],
        prompts: list[str],
        *,
        run_id: UUID,
        parent_run_id: UUID | None = None,
        **kwargs: Any,
    ) -> None:
        self._on_model_start(serialized, run_id, parent_run_id, kwargs, prompt_count=len(prompts))

    def on_chat_model_start(
        self,
        serialized: dict[str, Any],
        messages: list[list[Any]],
        *,
        run_id: UUID,
        parent_run_id: UUID | None = None,
        **kwargs: Any,
    ) -> None:
        prompt_count = sum(len(batch) for batch in messages)
        self._on_model_start(serialized, run_id, parent_run_id, kwargs, prompt_count=prompt_count)

    def _on_model_start(
        self,
        serialized: dict[str, Any] | None,
        run_id: UUID,
        parent_run_id: UUID | None,
        kwargs: dict[str, Any],
        *,
        prompt_count: int,
    ) -> None:
        request_model = _request_model(serialized, kwargs)
        span_name = f"chat {request_model}" if request_model else "chat"
        span = self._start_span(run_id, parent_run_id, span_name, SpanKind.CLIENT)

        params = _request_params(kwargs)
        attrs = GenAIAttributes(
            system=_infer_system(serialized, kwargs),
            operation_name="chat",
            request_model=request_model,
            temperature=_as_float(params.get("temperature")),
            top_p=_as_float(params.get("top_p")),
            top_k=_as_int(params.get("top_k")),
            max_tokens=_as_int(params.get("max_tokens") or params.get("max_tokens_to_sample")),
            seed=_as_int(params.get("seed")),
        )
        set_gen_ai_attributes(span, attrs)
        span.set_attribute("gen_ai.request.prompt_count", prompt_count)

    def on_llm_end(self, response: Any, *, run_id: UUID, **kwargs: Any) -> None:
        span = self._pop_span(run_id)
        if span is None:
            return
        try:
            llm_output = getattr(response, "llm_output", None)
            input_tokens, output_tokens, cached_tokens = _extract_usage(llm_output)
            response_model, response_id = _response_model_and_id(response)
            attrs = GenAIAttributes(
                response_model=response_model,
                response_id=response_id,
                usage_input_tokens=input_tokens,
                usage_output_tokens=output_tokens,
                usage_cached_tokens=cached_tokens,
                finish_reason=_extract_finish_reason(response),
            )
            set_gen_ai_attributes(span, attrs)
            span.set_status(Status(StatusCode.OK))
        finally:
            span.end()

    def on_llm_error(self, error: BaseException, *, run_id: UUID, **kwargs: Any) -> None:
        self._fail_span(run_id, error)

    # --- chains (LangChain Runnables / LangGraph nodes) -----------------------

    def on_chain_start(
        self,
        serialized: dict[str, Any] | None,
        inputs: dict[str, Any] | Any,
        *,
        run_id: UUID,
        parent_run_id: UUID | None = None,
        **kwargs: Any,
    ) -> None:
        name = None
        if serialized:
            name = serialized.get("name")
            if not name:
                cls_id = serialized.get("id")
                if isinstance(cls_id, list) and cls_id:
                    name = str(cls_id[-1])
        name = name or kwargs.get("name") or "chain"
        span = self._start_span(run_id, parent_run_id, f"chain {name}", SpanKind.INTERNAL)
        span.set_attribute("gen_ai.operation.name", "chain")
        span.set_attribute("langchain.chain.name", str(name))

    def on_chain_end(self, outputs: Any, *, run_id: UUID, **kwargs: Any) -> None:
        span = self._pop_span(run_id)
        if span is None:
            return
        span.set_status(Status(StatusCode.OK))
        span.end()

    def on_chain_error(self, error: BaseException, *, run_id: UUID, **kwargs: Any) -> None:
        self._fail_span(run_id, error)

    # --- tools ----------------------------------------------------------------

    def on_tool_start(
        self,
        serialized: dict[str, Any] | None,
        input_str: str,
        *,
        run_id: UUID,
        parent_run_id: UUID | None = None,
        **kwargs: Any,
    ) -> None:
        tool_name = None
        if serialized:
            tool_name = serialized.get("name")
        tool_name = tool_name or kwargs.get("name") or "tool"
        span = self._start_span(run_id, parent_run_id, f"tool {tool_name}", SpanKind.INTERNAL)
        set_gen_ai_attributes(span, GenAIAttributes(operation_name="tool", tool_names=[str(tool_name)]))
        span.set_attribute("gen_ai.tool.name", str(tool_name))
        if self._capture_content and input_str:
            span.set_attribute("gen_ai.tool.input", str(input_str))

    def on_tool_end(self, output: Any, *, run_id: UUID, **kwargs: Any) -> None:
        span = self._pop_span(run_id)
        if span is None:
            return
        if self._capture_content and output is not None:
            span.set_attribute("gen_ai.tool.output", str(output))
        span.set_status(Status(StatusCode.OK))
        span.end()

    def on_tool_error(self, error: BaseException, *, run_id: UUID, **kwargs: Any) -> None:
        self._fail_span(run_id, error)

    # --- shared error path ----------------------------------------------------

    def _fail_span(self, run_id: UUID, error: BaseException) -> None:
        span = self._pop_span(run_id)
        if span is None:
            return
        try:
            span.record_exception(error)
            span.set_status(Status(StatusCode.ERROR, str(error)))
        finally:
            span.end()


__all__ = ["SmooAICallbackHandler", "DEFAULT_SYSTEM"]
