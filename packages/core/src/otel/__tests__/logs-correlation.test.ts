import { context, trace } from '@opentelemetry/api';
import { logs } from '@opentelemetry/api-logs';
import { AsyncHooksContextManager } from '@opentelemetry/context-async-hooks';
import { InMemoryLogRecordExporter, LoggerProvider, SimpleLogRecordProcessor } from '@opentelemetry/sdk-logs';
import { BasicTracerProvider } from '@opentelemetry/sdk-trace-base';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';

// End-to-end proof of the trace↔log correlation the whole logs signal rides
// on: a log emitted through the standard `@opentelemetry/api-logs` facade
// (the same facade @smooai/logger bridges to) while a span is active MUST
// carry that span's real W3C trace_id / span_id. If it doesn't, every logger
// line lands uncorrelated in the product and the feature is pointless — so we
// exercise the real LoggerProvider + a real active span, not a stub.
describe('logs ↔ trace correlation via the api-logs facade', () => {
    const memoryExporter = new InMemoryLogRecordExporter();
    const loggerProvider = new LoggerProvider();
    const tracerProvider = new BasicTracerProvider();
    const contextManager = new AsyncHooksContextManager();

    beforeAll(() => {
        loggerProvider.addLogRecordProcessor(new SimpleLogRecordProcessor(memoryExporter));
        logs.setGlobalLoggerProvider(loggerProvider);
        // Without a real ContextManager `context.with(...)` is a noop and no
        // span is ever "active" — register async-hooks so the span propagates.
        context.setGlobalContextManager(contextManager.enable());
    });

    afterEach(() => {
        memoryExporter.getFinishedLogRecords().length = 0;
    });

    afterAll(() => {
        contextManager.disable();
    });

    it('stamps the active span trace_id + span_id onto a log emitted inside it', () => {
        const tracer = tracerProvider.getTracer('test');
        const span = tracer.startSpan('unit-of-work');
        const expected = span.spanContext();

        context.with(trace.setSpan(context.active(), span), () => {
            logs.getLogger('@smooai/logger').emit({ severityText: 'info', body: 'inside the span' });
        });
        span.end();

        const records = memoryExporter.getFinishedLogRecords();
        expect(records).toHaveLength(1);
        expect(records[0]!.spanContext?.traceId).toBe(expected.traceId);
        expect(records[0]!.spanContext?.spanId).toBe(expected.spanId);
        // Real W3C shapes — 32 / 16 lowercase hex.
        expect(records[0]!.spanContext?.traceId).toMatch(/^[0-9a-f]{32}$/);
        expect(records[0]!.spanContext?.spanId).toMatch(/^[0-9a-f]{16}$/);
    });

    it('emits without a span context when no span is active (graceful, still a record)', () => {
        logs.getLogger('@smooai/logger').emit({ severityText: 'warn', body: 'no span here' });

        const records = memoryExporter.getFinishedLogRecords();
        expect(records).toHaveLength(1);
        expect(records[0]!.spanContext).toBeUndefined();
        expect(records[0]!.body).toBe('no span here');
    });
});
