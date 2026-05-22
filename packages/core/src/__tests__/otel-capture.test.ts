import { context, ROOT_CONTEXT, SpanStatusCode, trace } from '@opentelemetry/api';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { BasicTracerProvider, InMemorySpanExporter, SimpleSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { Client } from '../client';
import { _resetOtelCaptureForTests, registerOtelCapture } from '../node/otel-capture';

const exporter = new InMemorySpanExporter();
const provider = new BasicTracerProvider({
    spanProcessors: [new SimpleSpanProcessor(exporter)],
});
trace.setGlobalTracerProvider(provider);
const cm = new AsyncHooksContextManager();
cm.enable();
context.setGlobalContextManager(cm);

describe('OTel-native captureException (node)', () => {
    beforeEach(() => {
        Client.init({ dsn: 'https://ingest.example/wh/o/t' });
        _resetOtelCaptureForTests();
        registerOtelCapture();
        exporter.reset();
    });

    afterEach(() => {
        _resetOtelCaptureForTests();
    });

    it('records the exception on the active span and marks status ERROR', () => {
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
        expect(spans[0]!.status.code).toBe(SpanStatusCode.ERROR);
        expect(spans[0]!.events.map((e) => e.name)).toContain('exception');
    });

    it('mints a synthetic span named observability.captureException when none active', () => {
        context.with(ROOT_CONTEXT, () => {
            Client.captureException(new Error('no-context boom'));
        });
        const spans = exporter.getFinishedSpans();
        expect(spans).toHaveLength(1);
        expect(spans[0]!.name).toBe('observability.captureException');
        expect(spans[0]!.status.code).toBe(SpanStatusCode.ERROR);
    });

    it('stamps the Smoo event id as smoo.event_id on the span', () => {
        const tracer = trace.getTracer('test');
        let eventId: string | undefined;
        tracer.startActiveSpan('handler', (span) => {
            eventId = Client.captureException(new Error('x'));
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        expect(span.attributes['smoo.event_id']).toBe(eventId);
    });

    it('propagates Smoo tags as smoo.tag.* attributes', () => {
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            Client.captureException(new Error('x'), { tags: { source: 'unit', tier: 'free' } });
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        expect(span.attributes['smoo.tag.source']).toBe('unit');
        expect(span.attributes['smoo.tag.tier']).toBe('free');
    });

    it('propagates Scope user as enduser.* attributes', () => {
        Client.setUser({ id: 'u1', orgId: 'org1', sessionId: 's1' });
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            Client.captureException(new Error('with user'));
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        expect(span.attributes['enduser.id']).toBe('u1');
        expect(span.attributes['enduser.org_id']).toBe('org1');
        expect(span.attributes['enduser.session_id']).toBe('s1');
        // Cleanup so other tests don't see this user.
        Client.setUser(undefined);
    });

    it('captureMessage adds a smoo.message span event without flipping status to ERROR', () => {
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            Client.captureMessage('hello', 'info');
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        expect(span.events.map((e) => e.name)).toContain('smoo.message');
        // Default status is UNSET (0); ERROR is 2.
        expect(span.status.code).not.toBe(SpanStatusCode.ERROR);
    });

    it("captureMessage with level='error' flips status to ERROR", () => {
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            Client.captureMessage('this failed', 'error');
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        expect(span.status.code).toBe(SpanStatusCode.ERROR);
    });

    it('is idempotent — registering twice does not double-capture', () => {
        registerOtelCapture();
        registerOtelCapture();
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            Client.captureException(new Error('once'));
            span.end();
        });
        const span = exporter.getFinishedSpans()[0]!;
        const exceptionEvents = span.events.filter((e) => e.name === 'exception');
        expect(exceptionEvents).toHaveLength(1);
    });

    it('fires BOTH the HTTP transport and the OTel capture when both are registered (SMOODEV-1148)', async () => {
        let transportCalled = 0;
        Client._registerTransport(async () => {
            transportCalled++;
        });
        // Register capture handler AFTER transport — both should now fire.
        _resetOtelCaptureForTests();
        registerOtelCapture();
        const tracer = trace.getTracer('test');
        tracer.startActiveSpan('handler', (span) => {
            Client.captureException(new Error('routed-to-both'));
            span.end();
        });
        // Allow microtask queue to drain.
        await new Promise((r) => setImmediate(r));
        // SMOODEV-1148: transport fires alongside the OTel capture so the
        // webhook-backed Errors dashboard sees Node errors. Prior behavior
        // was "captureHandler short-circuits transport" — see client.ts
        // and node/index.ts for rationale.
        expect(transportCalled).toBe(1);
        const spans = exporter.getFinishedSpans();
        expect(spans).toHaveLength(1);
    });
});
