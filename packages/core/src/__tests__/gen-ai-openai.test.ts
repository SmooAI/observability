import { context, SpanStatusCode, trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { BasicTracerProvider, InMemorySpanExporter, SimpleSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { wrapOpenAI } from '../gen-ai-openai';

const exporter = new InMemorySpanExporter();
trace.setGlobalTracerProvider(new BasicTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] }));
const cm = new AsyncHooksContextManager();
cm.enable();
context.setGlobalContextManager(cm);

/** Stand-in for the OpenAI Node client — same `chat.completions.create` shape. */
function fakeClient(create: (body: unknown) => unknown) {
    return { chat: { completions: { create: vi.fn(create) } }, apiKey: 'sk-not-real', baseURL: 'https://api.openai.com/v1' };
}

const RESPONSE = {
    id: 'chatcmpl-abc',
    model: 'gpt-4o-2024-11-20',
    choices: [{ finish_reason: 'stop', message: { role: 'assistant', content: 'four' } }],
    usage: { prompt_tokens: 11, completion_tokens: 3, prompt_tokens_details: { cached_tokens: 8 } },
};

const REQUEST = {
    model: 'gpt-4o',
    messages: [{ role: 'user', content: 'what is 2+2' }],
    temperature: 0.2,
    top_p: 0.95,
    max_tokens: 64,
    seed: 7,
    user: 'user_from_body',
    tools: [{ type: 'function', function: { name: 'calculator' } }],
};

function lastSpan() {
    const spans = exporter.getFinishedSpans();
    return spans[spans.length - 1]!;
}

describe('wrapOpenAI — non-streaming', () => {
    beforeEach(() => exporter.reset());

    it('emits the request-side keys off the call body', async () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        await client.chat.completions.create(REQUEST);

        const { attributes } = lastSpan();
        expect(attributes['gen_ai.system']).toBe('openai');
        expect(attributes['gen_ai.request.model']).toBe('gpt-4o');
        expect(attributes['gen_ai.request.temperature']).toBe(0.2);
        expect(attributes['gen_ai.request.top_p']).toBe(0.95);
        expect(attributes['gen_ai.request.max_tokens']).toBe(64);
        expect(attributes['gen_ai.request.seed']).toBe(7);
        expect(attributes['gen_ai.tool.names']).toEqual(['calculator']);
        expect(attributes['gen_ai.end_user.id']).toBe('user_from_body');
    });

    it('always sets gen_ai.operation.name — ingest has no fallback for it', async () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        await client.chat.completions.create({ model: 'gpt-4o', messages: [] });
        expect(lastSpan().attributes['gen_ai.operation.name']).toBe('chat');
    });

    it('emits the response-side keys off the provider response', async () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        await client.chat.completions.create(REQUEST);

        const { attributes } = lastSpan();
        expect(attributes['gen_ai.response.model']).toBe('gpt-4o-2024-11-20');
        expect(attributes['gen_ai.response.id']).toBe('chatcmpl-abc');
        expect(attributes['gen_ai.response.finish_reason']).toBe('stop');
        expect(attributes['gen_ai.usage.input_tokens']).toBe(11);
        expect(attributes['gen_ai.usage.output_tokens']).toBe(3);
        expect(attributes['gen_ai.usage.cached_tokens']).toBe(8);
        expect(attributes['gen_ai.response.truncated']).toBe(false);
    });

    it('marks truncated when the provider stopped on length', async () => {
        const client = wrapOpenAI(fakeClient(() => ({ ...RESPONSE, choices: [{ finish_reason: 'length' }] })));
        await client.chat.completions.create(REQUEST);
        expect(lastSpan().attributes['gen_ai.response.truncated']).toBe(true);
    });

    it('names the span "chat {model}" per semconv', async () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        await client.chat.completions.create(REQUEST);
        expect(lastSpan().name).toBe('chat gpt-4o');
    });

    it('returns the provider response untouched', async () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        await expect(client.chat.completions.create(REQUEST)).resolves.toBe(RESPONSE);
    });

    it('passes every other client property straight through', () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        expect(client.baseURL).toBe('https://api.openai.com/v1');
    });

    it('overrides the system for an OpenAI-compatible gateway', async () => {
        const client = wrapOpenAI(
            fakeClient(() => RESPONSE),
            { system: 'groq' },
        );
        await client.chat.completions.create(REQUEST);
        expect(lastSpan().attributes['gen_ai.system']).toBe('groq');
    });

    it('stamps the conversation and end-user ids from options', async () => {
        const client = wrapOpenAI(
            fakeClient(() => RESPONSE),
            { conversationId: 'conv_1', endUserId: 'user_1' },
        );
        await client.chat.completions.create(REQUEST);
        expect(lastSpan().attributes['gen_ai.conversation.id']).toBe('conv_1');
        expect(lastSpan().attributes['gen_ai.end_user.id']).toBe('user_1');
    });
});

describe('wrapOpenAI — cost seam', () => {
    beforeEach(() => exporter.reset());

    it('emits gen_ai.usage.cost_usd from the caller-supplied pricer', async () => {
        const costUsd = vi.fn(({ inputTokens, outputTokens }) => (inputTokens ?? 0) * 0.001 + (outputTokens ?? 0) * 0.002);
        const client = wrapOpenAI(
            fakeClient(() => RESPONSE),
            { costUsd },
        );
        await client.chat.completions.create(REQUEST);

        expect(costUsd).toHaveBeenCalledWith({ requestModel: 'gpt-4o', responseModel: 'gpt-4o-2024-11-20', inputTokens: 11, outputTokens: 3, cachedTokens: 8 });
        expect(lastSpan().attributes['gen_ai.usage.cost_usd']).toBeCloseTo(0.017);
    });

    it('leaves the cost attribute unset when no pricer is supplied', async () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        await client.chat.completions.create(REQUEST);
        expect(lastSpan().attributes['gen_ai.usage.cost_usd']).toBeUndefined();
    });
});

describe('wrapOpenAI — content recording', () => {
    beforeEach(() => exporter.reset());

    it('records no prompt or completion content by default', async () => {
        const client = wrapOpenAI(fakeClient(() => RESPONSE));
        await client.chat.completions.create(REQUEST);
        expect(lastSpan().events).toHaveLength(0);
    });

    it('records prompt and completion as gen_ai.*.message events when opted in', async () => {
        const client = wrapOpenAI(
            fakeClient(() => RESPONSE),
            { recordContent: true },
        );
        await client.chat.completions.create(REQUEST);

        const events = lastSpan().events;
        expect(events.map((e) => e.name)).toEqual(['gen_ai.user.message', 'gen_ai.assistant.message']);
        expect(events[1]!.attributes!['gen_ai.message.content']).toBe('four');
    });

    it('scrubs credentials out of recorded prompt content', async () => {
        const client = wrapOpenAI(
            fakeClient(() => RESPONSE),
            { recordContent: true },
        );
        await client.chat.completions.create({ model: 'gpt-4o', messages: [{ role: 'user', content: 'my key is sk-abcdefghijklmnopqrstuvwxyz' }] });
        expect(lastSpan().events[0]!.attributes!['gen_ai.message.content']).not.toContain('abcdefghijklmnopqrstuvwxyz');
    });
});

describe('wrapOpenAI — errors', () => {
    beforeEach(() => exporter.reset());

    it('marks the span ERROR and rethrows the provider failure', async () => {
        const client = wrapOpenAI(
            fakeClient(() => {
                throw new Error('429 rate limited');
            }),
        );
        await expect(client.chat.completions.create(REQUEST)).rejects.toThrow('429 rate limited');

        const span = lastSpan();
        expect(span.status.code).toBe(SpanStatusCode.ERROR);
        expect(span.events.map((e) => e.name)).toContain('exception');
        // Request attributes still land, so a failed call is still attributable.
        expect(span.attributes['gen_ai.request.model']).toBe('gpt-4o');
    });
});

/** Minimal stand-in for the OpenAI `Stream` object: async-iterable, plus extras. */
function fakeStream(chunks: unknown[]) {
    return {
        controller: { abort: () => {} },
        async *[Symbol.asyncIterator]() {
            for (const c of chunks) yield c;
        },
    };
}

const STREAM_CHUNKS = [
    { id: 'chatcmpl-s', model: 'gpt-4o-2024-11-20', choices: [{ delta: { content: 'fo' } }] },
    { id: 'chatcmpl-s', model: 'gpt-4o-2024-11-20', choices: [{ delta: { content: 'ur' }, finish_reason: 'stop' }] },
    { id: 'chatcmpl-s', model: 'gpt-4o-2024-11-20', choices: [], usage: { prompt_tokens: 11, completion_tokens: 3 } },
];

describe('wrapOpenAI — streaming', () => {
    beforeEach(() => exporter.reset());

    it('keeps the span open until the stream drains, then records usage', async () => {
        const client = wrapOpenAI(fakeClient(() => fakeStream(STREAM_CHUNKS)));
        const stream = await client.chat.completions.create({ ...REQUEST, stream: true });

        expect(exporter.getFinishedSpans()).toHaveLength(0);
        const seen = [];
        for await (const chunk of stream as AsyncIterable<unknown>) seen.push(chunk);

        expect(seen).toHaveLength(3);
        const { attributes } = lastSpan();
        expect(attributes['gen_ai.response.id']).toBe('chatcmpl-s');
        expect(attributes['gen_ai.response.model']).toBe('gpt-4o-2024-11-20');
        expect(attributes['gen_ai.response.finish_reason']).toBe('stop');
        expect(attributes['gen_ai.usage.input_tokens']).toBe(11);
        expect(attributes['gen_ai.usage.output_tokens']).toBe(3);
    });

    it('ends the span when the consumer breaks out early', async () => {
        const client = wrapOpenAI(fakeClient(() => fakeStream(STREAM_CHUNKS)));
        const stream = await client.chat.completions.create({ ...REQUEST, stream: true });
        for await (const _chunk of stream as AsyncIterable<unknown>) break;
        expect(exporter.getFinishedSpans()).toHaveLength(1);
    });

    it('preserves non-iterator members of the Stream object', async () => {
        const client = wrapOpenAI(fakeClient(() => fakeStream(STREAM_CHUNKS)));
        const stream = (await client.chat.completions.create({ ...REQUEST, stream: true })) as { controller: unknown };
        expect(stream.controller).toBeDefined();
    });

    it('assembles streamed deltas into one assistant message when opted in', async () => {
        const client = wrapOpenAI(
            fakeClient(() => fakeStream(STREAM_CHUNKS)),
            { recordContent: true },
        );
        const stream = await client.chat.completions.create({ ...REQUEST, stream: true });
        for await (const _chunk of stream as AsyncIterable<unknown>) {
            /* drain */
        }
        const assistant = lastSpan().events.find((e) => e.name === 'gen_ai.assistant.message');
        expect(assistant!.attributes!['gen_ai.message.content']).toBe('four');
    });
});
