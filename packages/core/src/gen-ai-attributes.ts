/**
 * OTel GenAI Semantic Conventions — attribute helpers.
 *
 * The OTel spec working group standardized `gen_ai.*` attributes for LLM /
 * agent telemetry so any OTel backend (Datadog, Honeycomb, Jaeger, Smoo's
 * own LLM dashboard) can interpret them with one shared vocabulary. This
 * module exports a typed helper for setting those attributes on spans
 * without hand-coding the constants in every caller.
 *
 * Spec reference: https://opentelemetry.io/docs/specs/semconv/gen-ai/
 *
 * SMOODEV-1155 — initial vocabulary. The spec is still moving (recent
 * additions cover tool calls + streaming + multi-agent loops); extend
 * `GenAIAttributes` as new attributes stabilize.
 */
import type { Span } from '@opentelemetry/api';

export type GenAIOperationName = 'chat' | 'text_completion' | 'embeddings' | 'tool' | 'agent' | 'rerank';

export type GenAISystem = 'openai' | 'anthropic' | 'groq' | 'gemini' | 'cohere' | 'mistral' | 'deepseek' | 'azure_openai' | 'aws_bedrock' | (string & {});

/**
 * Full set of GenAI semantic-convention attributes. All optional — set what
 * you know. Used by `setGenAIAttributes(span, attrs)` to apply onto an
 * active OTel span. Keys mirror the spec exactly so backends parse without
 * mapping.
 */
export interface GenAIAttributes {
    /** Provider system, e.g. 'openai', 'anthropic'. */
    system?: GenAISystem;
    /** Operation kind. Pick 'agent' for top-level agent spans, 'chat' for LLM calls, 'tool' for tool invocations. */
    operationName?: GenAIOperationName;

    /** Model the request asked for, e.g. 'gpt-4o', 'claude-opus-4-7'. */
    requestModel?: string;
    /** Model the response actually came back from (may differ from requested under fallback). */
    responseModel?: string;
    /** Provider-issued ID of the response (Anthropic message ID, OpenAI completion ID, etc.). */
    responseId?: string;

    /** Sampling temperature. */
    temperature?: number;
    /** Top-P sampling cutoff. */
    topP?: number;
    /** Top-K sampling cutoff. */
    topK?: number;
    /** Max generated token budget. */
    maxTokens?: number;
    /** Random seed if supplied. */
    seed?: number;

    /** Input/prompt tokens billed. */
    usageInputTokens?: number;
    /** Output/completion tokens billed. */
    usageOutputTokens?: number;
    /** Cached-prompt tokens (Anthropic prompt-cache hit / OpenAI cached input). */
    usageCachedTokens?: number;
    /** Total cost in USD (filled in by the platform from upstream provider invoices when available). */
    usageCostUsd?: number;

    /** Comma-separated list of tool names called within this span. */
    toolNames?: string[];
    /** Truncation flag — true if the response was cut off by max_tokens / stop_reason. */
    truncated?: boolean;
    /** Provider's reported finish reason ('stop', 'length', 'tool_calls', etc.). */
    finishReason?: string;

    /** End-user id from your system (NOT the provider's id). */
    endUserId?: string;
    /** Conversation/session id. */
    conversationId?: string;
}

/**
 * Apply a {@link GenAIAttributes} value onto the active OTel span. Skips
 * undefined fields so partial calls (e.g. setting just the model + tokens
 * after a streaming completion finishes) are idempotent.
 */
export function setGenAIAttributes(span: Span, attrs: GenAIAttributes): void {
    if (attrs.system !== undefined) span.setAttribute('gen_ai.system', attrs.system);
    if (attrs.operationName !== undefined) span.setAttribute('gen_ai.operation.name', attrs.operationName);
    if (attrs.requestModel !== undefined) span.setAttribute('gen_ai.request.model', attrs.requestModel);
    if (attrs.responseModel !== undefined) span.setAttribute('gen_ai.response.model', attrs.responseModel);
    if (attrs.responseId !== undefined) span.setAttribute('gen_ai.response.id', attrs.responseId);
    if (attrs.temperature !== undefined) span.setAttribute('gen_ai.request.temperature', attrs.temperature);
    if (attrs.topP !== undefined) span.setAttribute('gen_ai.request.top_p', attrs.topP);
    if (attrs.topK !== undefined) span.setAttribute('gen_ai.request.top_k', attrs.topK);
    if (attrs.maxTokens !== undefined) span.setAttribute('gen_ai.request.max_tokens', attrs.maxTokens);
    if (attrs.seed !== undefined) span.setAttribute('gen_ai.request.seed', attrs.seed);
    if (attrs.usageInputTokens !== undefined) span.setAttribute('gen_ai.usage.input_tokens', attrs.usageInputTokens);
    if (attrs.usageOutputTokens !== undefined) span.setAttribute('gen_ai.usage.output_tokens', attrs.usageOutputTokens);
    if (attrs.usageCachedTokens !== undefined) span.setAttribute('gen_ai.usage.cached_tokens', attrs.usageCachedTokens);
    if (attrs.usageCostUsd !== undefined) span.setAttribute('gen_ai.usage.cost_usd', attrs.usageCostUsd);
    if (attrs.toolNames !== undefined && attrs.toolNames.length > 0) span.setAttribute('gen_ai.tool.names', attrs.toolNames);
    if (attrs.truncated !== undefined) span.setAttribute('gen_ai.response.truncated', attrs.truncated);
    if (attrs.finishReason !== undefined) span.setAttribute('gen_ai.response.finish_reason', attrs.finishReason);
    if (attrs.endUserId !== undefined) span.setAttribute('gen_ai.end_user.id', attrs.endUserId);
    if (attrs.conversationId !== undefined) span.setAttribute('gen_ai.conversation.id', attrs.conversationId);
}

/**
 * Emit a `gen_ai.user.message` / `gen_ai.assistant.message` / `gen_ai.system.message`
 * span event so the dashboard's prompt/completion side-by-side view can render
 * the actual content of the LLM call. Use sparingly — these are size-heavy.
 */
export function recordGenAIMessage(
    span: Span,
    role: 'user' | 'assistant' | 'system' | 'tool',
    content: string,
    extra?: { toolCallId?: string; toolName?: string },
): void {
    const eventName = `gen_ai.${role}.message`;
    span.addEvent(eventName, {
        'gen_ai.message.content': content,
        ...(extra?.toolCallId !== undefined && { 'gen_ai.tool_call.id': extra.toolCallId }),
        ...(extra?.toolName !== undefined && { 'gen_ai.tool.name': extra.toolName }),
    });
}
