import { context, ROOT_CONTEXT, SpanStatusCode, trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { BasicTracerProvider, InMemorySpanExporter, SimpleSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { Client } from '@smooai/observability';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { _resetBridgeForTests, bridgeClientToOtel, readOtelCorrelation } from '../bridge-to-client';

const exporter = new InMemorySpanExporter();
const provider = new BasicTracerProvider({
    spanProcessors: [new SimpleSpanProcessor(exporter)],
});
trace.setGlobalTracerProvider(provider);
// Without a registered context manager, OTel's default no-op manager makes
// startActiveSpan a no-op for `getActiveSpan()`. AsyncHooksContextManager is
// the production-equivalent here.
const contextManager = new AsyncHooksContextManager();
contextManager.enable();
context.setGlobalContextManager(contextManager);

describe('bridgeClientToOtel', () => {
    beforeEach(() => {
        Client.init({ dsn: 'https://ingest.example/wh/o/t' });
        _resetBridgeForTests();
        exporter.reset();
    });

    afterEach(() => {
        _resetBridgeForTests();
    });

    it('records the exception on the active span and marks status ERROR', () => {
        bridgeClientToOtel();
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            try {
                Client.captureException(new Error('boom'));
            } finally {
                span.end();
            }
        });
        const spans = exporter.getFinishedSpans();
        expect(spans).toHaveLength(1);
        const handlerSpan = spans[0]!;
        expect(handlerSpan.status.code).toBe(SpanStatusCode.ERROR);
        expect(handlerSpan.events.map((e) => e.name)).toContain('exception');
    });

    it('mints a synthetic span when no span is active', () => {
        bridgeClientToOtel();
        // Force into ROOT_CONTEXT so no active span is present.
        context.with(ROOT_CONTEXT, () => {
            Client.captureException(new Error('no-context boom'));
        });
        const spans = exporter.getFinishedSpans();
        expect(spans).toHaveLength(1);
        expect(spans[0]!.name).toBe('observability.captureException');
        expect(spans[0]!.status.code).toBe(SpanStatusCode.ERROR);
    });

    it('propagates Smoo event id onto the span as an attribute', () => {
        bridgeClientToOtel();
        const tracer = trace.getTracer('test');
        let eventId: string | undefined;
        tracer.startActiveSpan('handler', (span) => {
            eventId = Client.captureException(new Error('x'));
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        expect(span.attributes['smoo.event_id']).toBe(eventId);
    });

    it('is idempotent — installing twice does not double-wrap', () => {
        bridgeClientToOtel();
        bridgeClientToOtel();
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            Client.captureException(new Error('once'));
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        // Two installs would record the exception twice.
        const exceptionEvents = span.events.filter((e) => e.name === 'exception');
        expect(exceptionEvents).toHaveLength(1);
    });

    it('readOtelCorrelation returns active trace/span ids', () => {
        const tracer = trace.getTracer('test');
        let traceId: string | undefined;
        let spanId: string | undefined;
        tracer.startActiveSpan('outer', (span) => {
            const corr = readOtelCorrelation();
            traceId = corr.traceId;
            spanId = corr.spanId;
            span.end();
        });
        expect(traceId).toMatch(/^[0-9a-f]{32}$/);
        expect(spanId).toMatch(/^[0-9a-f]{16}$/);
    });

    it('readOtelCorrelation returns empty when no span active', () => {
        context.with(ROOT_CONTEXT, () => {
            const corr = readOtelCorrelation();
            expect(corr).toEqual({});
        });
    });

    it('bridge does not throw if Client.captureException throws internally', () => {
        bridgeClientToOtel();
        const orig = Client.captureException;
        (Client as unknown as { captureException: (...args: unknown[]) => unknown }).captureException = () => {
            throw new Error('transport down');
        };
        const tracer = trace.getTracer('test');
        // Wrap restoration so the rest of the suite still works.
        try {
            tracer.startActiveSpan('handler', (span) => {
                expect(() => Client.captureException(new Error('outer'))).toThrow('transport down');
                span.end();
            });
        } finally {
            (Client as unknown as { captureException: typeof orig }).captureException = orig;
        }
    });
});
