/**
 * OpenAI Node SDK instrumentation — one wrapper that emits OTel GenAI
 * semantic-convention spans for every `chat.completions.create` call.
 *
 * Why the OpenAI client and not the Vercel AI SDK: the OpenAI *wire shape* is
 * the lingua franca. Groq, Together, Fireworks, DeepSeek, Azure OpenAI, and our
 * own LiteLLM gateway at `llm.smoo.ai` all speak it through this same client, so
 * one wrapper instruments all of them. It also needs no new dependency — the
 * client is duck-typed structurally below, so `@smooai/observability` never
 * imports `openai`.
 *
 * ```ts
 * import OpenAI from 'openai';
 * import { wrapOpenAI } from '@smooai/observability';
 *
 * const client = wrapOpenAI(new OpenAI(), { conversationId: convo.id });
 * await client.chat.completions.create({ model: 'gpt-4o', messages });
 * ```
 *
 * Spec: https://opentelemetry.io/docs/specs/semconv/gen-ai/
 */
import { SpanKind, SpanStatusCode, trace, type Span } from '@opentelemetry/api';
import { recordGenAIMessage, setGenAIAttributes, type GenAIAttributes, type GenAISystem } from './gen-ai-attributes';

const TRACER_NAME = '@smooai/observability/gen-ai';

/** Token counts pulled off the provider response, handed to {@link WrapOpenAIOptions.costUsd}. */
export interface GenAICostInput {
    requestModel?: string;
    responseModel?: string;
    inputTokens?: number;
    outputTokens?: number;
    cachedTokens?: number;
}

export interface WrapOpenAIOptions {
    /**
     * `gen_ai.system` value. Defaults to `'openai'`. Set it when pointing the
     * client at an OpenAI-compatible gateway (`'groq'`, `'deepseek'`, …) so the
     * spans attribute to the real provider.
     */
    system?: GenAISystem;
    /** `gen_ai.conversation.id` — stamped on every span from this client. */
    conversationId?: string;
    /** `gen_ai.end_user.id`. Falls back to the request body's `user` field. */
    endUserId?: string;
    /**
     * Cost seam. Nothing in the platform emits `gen_ai.usage.cost_usd` on its
     * own — providers don't return a price — so the dashboard's cost column
     * stays empty until a caller supplies one. Return `undefined` to leave the
     * attribute unset.
     */
    costUsd?: (input: GenAICostInput) => number | undefined;
    /**
     * Record prompt + completion text as `gen_ai.*.message` span events.
     * **Off by default**: this is the SDK's most PII-dense payload, and events
     * are size-heavy. Content still goes through the SDK's PII scrub when on.
     */
    recordContent?: boolean;
}

/** Minimal structural view of the OpenAI client — avoids depending on `openai`. */
type AnyRecord = Record<string, unknown>;

function num(value: unknown): number | undefined {
    return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function str(value: unknown): string | undefined {
    return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function asRecord(value: unknown): AnyRecord | undefined {
    return typeof value === 'object' && value !== null ? (value as AnyRecord) : undefined;
}

function isAsyncIterable(value: unknown): value is AsyncIterable<unknown> {
    return typeof (value as AsyncIterable<unknown> | undefined)?.[Symbol.asyncIterator] === 'function';
}

/** Tool names off an OpenAI `tools: [{ type: 'function', function: { name } }]` array. */
function toolNames(body: AnyRecord): string[] | undefined {
    const tools = body.tools;
    if (!Array.isArray(tools)) return undefined;
    const names = tools.map((t) => str(asRecord(asRecord(t)?.function)?.name) ?? str(asRecord(t)?.name)).filter((n): n is string => n !== undefined);
    return names.length > 0 ? names : undefined;
}

/** Request-side attributes, all of which are known before the call goes out. */
function requestAttributes(body: AnyRecord, options: WrapOpenAIOptions): GenAIAttributes {
    return {
        system: options.system ?? 'openai',
        // Straight passthrough on ingest with NO fallback — unset here means a
        // NULL operation column in `gen_ai_events`, so always set it.
        operationName: 'chat',
        requestModel: str(body.model),
        temperature: num(body.temperature),
        topP: num(body.top_p),
        maxTokens: num(body.max_tokens) ?? num(body.max_completion_tokens),
        seed: num(body.seed),
        toolNames: toolNames(body),
        endUserId: options.endUserId ?? str(body.user),
        conversationId: options.conversationId,
    };
}

/** Usage block, tolerating the chat-completions and cached-token shapes. */
function usageAttributes(usage: AnyRecord | undefined): Pick<GenAIAttributes, 'usageInputTokens' | 'usageOutputTokens' | 'usageCachedTokens'> {
    if (!usage) return {};
    return {
        usageInputTokens: num(usage.prompt_tokens) ?? num(usage.input_tokens),
        usageOutputTokens: num(usage.completion_tokens) ?? num(usage.output_tokens),
        usageCachedTokens: num(asRecord(usage.prompt_tokens_details)?.cached_tokens) ?? num(asRecord(usage.input_tokens_details)?.cached_tokens),
    };
}

/** Apply everything the provider told us after the call resolved. */
function applyResponse(span: Span, requestModel: string | undefined, response: AnyRecord, options: WrapOpenAIOptions): void {
    const usage = usageAttributes(asRecord(response.usage));
    const finishReason = str(asRecord(asRecord(response.choices)?.[0] as unknown)?.finish_reason);
    const responseModel = str(response.model);

    setGenAIAttributes(span, {
        ...usage,
        responseModel,
        responseId: str(response.id),
        finishReason,
        truncated: finishReason === undefined ? undefined : finishReason === 'length',
        usageCostUsd: options.costUsd?.({
            requestModel,
            responseModel,
            inputTokens: usage.usageInputTokens,
            outputTokens: usage.usageOutputTokens,
            cachedTokens: usage.usageCachedTokens,
        }),
    });

    if (options.recordContent) {
        const message = asRecord(asRecord(asRecord(response.choices)?.[0] as unknown)?.message);
        const content = str(message?.content);
        if (content !== undefined) recordGenAIMessage(span, 'assistant', content);
    }
}

/** Record the prompt messages the caller sent, when content recording is on. */
function recordPrompt(span: Span, body: AnyRecord): void {
    const messages = body.messages;
    if (!Array.isArray(messages)) return;
    for (const raw of messages) {
        const message = asRecord(raw);
        const content = str(message?.content);
        const role = str(message?.role);
        if (content === undefined || role === undefined) continue;
        if (role !== 'user' && role !== 'assistant' && role !== 'system' && role !== 'tool') continue;
        recordGenAIMessage(span, role, content, { toolCallId: str(message?.tool_call_id) });
    }
}

function failSpan(span: Span, error: unknown): void {
    span.recordException(error instanceof Error ? error : new Error(String(error)));
    span.setStatus({ code: SpanStatusCode.ERROR, message: error instanceof Error ? error.message : String(error) });
}

/**
 * Wrap a streaming response so the span stays open for the whole stream and
 * closes when the consumer finishes or breaks out early. Proxied rather than
 * replaced so `stream.controller` / `stream.tee()` keep working.
 */
function instrumentStream(stream: AsyncIterable<unknown>, span: Span, requestModel: string | undefined, options: WrapOpenAIOptions): AsyncIterable<unknown> {
    return new Proxy(stream as object, {
        get(target, prop, receiver) {
            if (prop !== Symbol.asyncIterator) {
                const value = Reflect.get(target, prop, receiver);
                return typeof value === 'function' ? value.bind(target) : value;
            }
            return function instrumentedIterator() {
                return (async function* () {
                    // Chunks carry the response identity in pieces; the last one
                    // with `usage` only appears under `stream_options.include_usage`.
                    const merged: AnyRecord = {};
                    let content = '';
                    try {
                        for await (const raw of stream) {
                            const chunk = asRecord(raw);
                            if (chunk) {
                                if (str(chunk.id) !== undefined) merged.id = chunk.id;
                                if (str(chunk.model) !== undefined) merged.model = chunk.model;
                                if (asRecord(chunk.usage) !== undefined) merged.usage = chunk.usage;
                                const choice = asRecord(asRecord(chunk.choices)?.[0] as unknown);
                                if (str(choice?.finish_reason) !== undefined) {
                                    merged.choices = [{ finish_reason: choice?.finish_reason }];
                                }
                                if (options.recordContent) content += str(asRecord(choice?.delta)?.content) ?? '';
                            }
                            yield raw;
                        }
                        if (options.recordContent && content.length > 0) {
                            merged.choices = [{ ...(asRecord(asRecord(merged.choices)?.[0] as unknown) ?? {}), message: { content } }];
                        }
                        applyResponse(span, requestModel, merged, options);
                    } catch (error) {
                        failSpan(span, error);
                        throw error;
                    } finally {
                        span.end();
                    }
                })();
            };
        },
    }) as AsyncIterable<unknown>;
}

function instrumentedCreate(create: (...args: unknown[]) => unknown, self: unknown, options: WrapOpenAIOptions) {
    return function wrappedCreate(this: unknown, ...args: unknown[]) {
        const body = asRecord(args[0]) ?? {};
        const attrs = requestAttributes(body, options);
        const spanName = attrs.requestModel !== undefined ? `chat ${attrs.requestModel}` : 'chat';

        return trace.getTracer(TRACER_NAME).startActiveSpan(spanName, { kind: SpanKind.CLIENT }, async (span) => {
            setGenAIAttributes(span, attrs);
            if (options.recordContent) recordPrompt(span, body);

            let streaming = false;
            try {
                const result = await create.apply(self ?? this, args);
                // A streaming call resolves to an async iterable; the span has to
                // outlive it, so ownership of `span.end()` moves to the iterator.
                if (body.stream === true && isAsyncIterable(result)) {
                    streaming = true;
                    return instrumentStream(result as AsyncIterable<unknown>, span, attrs.requestModel, options);
                }
                const response = asRecord(result);
                if (response) applyResponse(span, attrs.requestModel, response, options);
                return result;
            } catch (error) {
                failSpan(span, error);
                throw error;
            } finally {
                if (!streaming) span.end();
            }
        });
    };
}

/**
 * Return a proxy of `client` whose `chat.completions.create` is traced. The
 * original client is untouched, and every other property passes through, so the
 * wrapper is safe to apply once at construction.
 *
 * ponytail: only `chat.completions.create` is instrumented. `embeddings`,
 * `responses`, and the Assistants API each need their own response shape —
 * add them when something actually calls them.
 */
export function wrapOpenAI<T extends object>(client: T, options: WrapOpenAIOptions = {}): T {
    return proxyPath(client, ['chat', 'completions', 'create'], (fn, self) => instrumentedCreate(fn, self, options));
}

/** Proxy just one property path on an object graph, leaving everything else alone. */
function proxyPath<T extends object>(target: T, path: readonly string[], wrap: (fn: (...args: unknown[]) => unknown, self: unknown) => unknown): T {
    const [head, ...rest] = path;
    return new Proxy(target, {
        get(obj, prop, receiver) {
            const value = Reflect.get(obj, prop, receiver);
            if (prop !== head) return value;
            if (rest.length === 0) {
                return typeof value === 'function' ? wrap(value as (...args: unknown[]) => unknown, obj) : value;
            }
            return typeof value === 'object' && value !== null ? proxyPath(value as object, rest, wrap) : value;
        },
    });
}
