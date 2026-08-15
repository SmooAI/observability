import { trace } from '@opentelemetry/api';
import { BasicTracerProvider, InMemorySpanExporter, SimpleSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { beforeEach, describe, expect, it } from 'vitest';
import { recordGenAIMessage, setGenAIAttributes, type GenAIAttributes } from '../gen-ai-attributes';

const exporter = new InMemorySpanExporter();
trace.setGlobalTracerProvider(new BasicTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] }));
const tracer = trace.getTracer('test');

/** Run `fn` against a fresh span and hand back the exported span. */
function onSpan(fn: (span: ReturnType<typeof tracer.startSpan>) => void) {
    const span = tracer.startSpan('llm');
    fn(span);
    span.end();
    const finished = exporter.getFinishedSpans();
    return finished[finished.length - 1]!;
}

/**
 * Every attribute `setGenAIAttributes` can emit, and the key it MUST emit it
 * under. A typo in any of these produces a span the platform cannot route, so
 * the keys are asserted literally rather than derived from the source.
 */
const EXPECTED_KEYS: Record<keyof GenAIAttributes, string> = {
    system: 'gen_ai.system',
    operationName: 'gen_ai.operation.name',
    requestModel: 'gen_ai.request.model',
    responseModel: 'gen_ai.response.model',
    responseId: 'gen_ai.response.id',
    temperature: 'gen_ai.request.temperature',
    topP: 'gen_ai.request.top_p',
    topK: 'gen_ai.request.top_k',
    maxTokens: 'gen_ai.request.max_tokens',
    seed: 'gen_ai.request.seed',
    usageInputTokens: 'gen_ai.usage.input_tokens',
    usageOutputTokens: 'gen_ai.usage.output_tokens',
    usageCachedTokens: 'gen_ai.usage.cached_tokens',
    usageCostUsd: 'gen_ai.usage.cost_usd',
    toolNames: 'gen_ai.tool.names',
    truncated: 'gen_ai.response.truncated',
    finishReason: 'gen_ai.response.finish_reason',
    endUserId: 'gen_ai.end_user.id',
    conversationId: 'gen_ai.conversation.id',
};

const FULL_ATTRS: Required<GenAIAttributes> = {
    system: 'anthropic',
    operationName: 'chat',
    requestModel: 'claude-opus-4-7',
    responseModel: 'claude-opus-4-7-20260101',
    responseId: 'msg_123',
    temperature: 0.7,
    topP: 0.9,
    topK: 40,
    maxTokens: 1024,
    seed: 42,
    usageInputTokens: 100,
    usageOutputTokens: 250,
    usageCachedTokens: 80,
    usageCostUsd: 0.0123,
    toolNames: ['search', 'calendar'],
    truncated: false,
    finishReason: 'stop',
    endUserId: 'user_abc',
    conversationId: 'conv_xyz',
};

describe('setGenAIAttributes', () => {
    beforeEach(() => exporter.reset());

    it('emits every attribute under its exact semantic-convention key', () => {
        const span = onSpan((s) => setGenAIAttributes(s, FULL_ATTRS));

        expect(span.attributes['gen_ai.system']).toBe('anthropic');
        expect(span.attributes['gen_ai.operation.name']).toBe('chat');
        expect(span.attributes['gen_ai.request.model']).toBe('claude-opus-4-7');
        expect(span.attributes['gen_ai.response.model']).toBe('claude-opus-4-7-20260101');
        expect(span.attributes['gen_ai.response.id']).toBe('msg_123');
        expect(span.attributes['gen_ai.request.temperature']).toBe(0.7);
        expect(span.attributes['gen_ai.request.top_p']).toBe(0.9);
        expect(span.attributes['gen_ai.request.top_k']).toBe(40);
        expect(span.attributes['gen_ai.request.max_tokens']).toBe(1024);
        expect(span.attributes['gen_ai.request.seed']).toBe(42);
        expect(span.attributes['gen_ai.usage.input_tokens']).toBe(100);
        expect(span.attributes['gen_ai.usage.output_tokens']).toBe(250);
        expect(span.attributes['gen_ai.usage.cached_tokens']).toBe(80);
        expect(span.attributes['gen_ai.usage.cost_usd']).toBe(0.0123);
        expect(span.attributes['gen_ai.tool.names']).toEqual(['search', 'calendar']);
        expect(span.attributes['gen_ai.response.truncated']).toBe(false);
        expect(span.attributes['gen_ai.response.finish_reason']).toBe('stop');
        expect(span.attributes['gen_ai.end_user.id']).toBe('user_abc');
        expect(span.attributes['gen_ai.conversation.id']).toBe('conv_xyz');
    });

    it('emits nothing outside the gen_ai.* vocabulary, and covers every field', () => {
        const span = onSpan((s) => setGenAIAttributes(s, FULL_ATTRS));
        expect(new Set(Object.keys(span.attributes))).toEqual(new Set(Object.values(EXPECTED_KEYS)));
    });

    it('skips undefined fields so partial calls are additive', () => {
        const span = onSpan((s) => {
            setGenAIAttributes(s, { system: 'openai', operationName: 'chat', requestModel: 'gpt-4o' });
            setGenAIAttributes(s, { usageInputTokens: 5, usageOutputTokens: 9 });
        });
        expect(Object.keys(span.attributes).sort()).toEqual([
            'gen_ai.operation.name',
            'gen_ai.request.model',
            'gen_ai.system',
            'gen_ai.usage.input_tokens',
            'gen_ai.usage.output_tokens',
        ]);
    });

    it('omits gen_ai.tool.names for an empty tool list', () => {
        const span = onSpan((s) => setGenAIAttributes(s, { toolNames: [] }));
        expect(span.attributes['gen_ai.tool.names']).toBeUndefined();
    });

    it('accepts an arbitrary system string for providers outside the known set', () => {
        const span = onSpan((s) => setGenAIAttributes(s, { system: 'my-private-gateway' }));
        expect(span.attributes['gen_ai.system']).toBe('my-private-gateway');
    });
});

/**
 * Routing contract with `rust/api-prime/src/handlers/observability/ingest_traces.rs`.
 * That handler forwards a span to `gen_ai_events` iff it carries `gen_ai.system`,
 * then reads exactly these columns off the merged attribute map. Every one is a
 * straight passthrough — `gen_ai.operation.name` in particular has NO fallback,
 * so a caller that leaves it unset writes a NULL operation column.
 */
const INGEST_READS = [
    'gen_ai.system',
    'gen_ai.operation.name',
    'gen_ai.request.model',
    'gen_ai.request.temperature',
    'gen_ai.request.top_p',
    'gen_ai.request.max_tokens',
    'gen_ai.response.model',
    'gen_ai.response.id',
    'gen_ai.response.finish_reason',
    'gen_ai.usage.input_tokens',
    'gen_ai.usage.output_tokens',
    'gen_ai.usage.cached_tokens',
    'gen_ai.usage.cost_usd',
    'gen_ai.end_user.id',
    'gen_ai.conversation.id',
] as const;

describe('ingest routing contract', () => {
    beforeEach(() => exporter.reset());

    it('produces every column ingest_traces.rs reads', () => {
        const span = onSpan((s) => setGenAIAttributes(s, FULL_ATTRS));
        for (const key of INGEST_READS) {
            expect(span.attributes[key], `ingest reads ${key} but the SDK never emitted it`).toBeDefined();
        }
    });

    it('carries the gen_ai.system trigger that routes the span at all', () => {
        // Without this exact key the span lands in ClickHouse traces only and
        // never reaches gen_ai_events — the whole LLM dashboard goes dark.
        const span = onSpan((s) => setGenAIAttributes(s, { system: 'openai' }));
        expect(Object.keys(span.attributes)).toContain('gen_ai.system');
    });
});

describe('recordGenAIMessage', () => {
    beforeEach(() => exporter.reset());

    it('names the event gen_ai.{role}.message and keys content exactly', () => {
        const span = onSpan((s) => recordGenAIMessage(s, 'assistant', 'hello there'));
        expect(span.events[0]!.name).toBe('gen_ai.assistant.message');
        expect(span.events[0]!.attributes!['gen_ai.message.content']).toBe('hello there');
    });

    it.each(['user', 'assistant', 'system', 'tool'] as const)('supports the %s role', (role) => {
        const span = onSpan((s) => recordGenAIMessage(s, role, 'x'));
        expect(span.events[0]!.name).toBe(`gen_ai.${role}.message`);
    });

    it('attaches tool linkage under the spec keys', () => {
        const span = onSpan((s) => recordGenAIMessage(s, 'tool', 'result', { toolCallId: 'call_1', toolName: 'search' }));
        expect(span.events[0]!.attributes!['gen_ai.tool_call.id']).toBe('call_1');
        expect(span.events[0]!.attributes!['gen_ai.tool.name']).toBe('search');
    });

    it('omits tool linkage keys when not supplied', () => {
        const span = onSpan((s) => recordGenAIMessage(s, 'user', 'hi'));
        expect(Object.keys(span.events[0]!.attributes!)).toEqual(['gen_ai.message.content']);
    });

    it('PII-scrubs message content before it leaves the process', () => {
        const span = onSpan((s) => recordGenAIMessage(s, 'user', 'call the api with Bearer abc123token and password=hunter2'));
        const content = span.events[0]!.attributes!['gen_ai.message.content'] as string;
        expect(content).not.toContain('abc123token');
        expect(content).not.toContain('hunter2');
        expect(content).toContain('[redacted]');
    });
});
