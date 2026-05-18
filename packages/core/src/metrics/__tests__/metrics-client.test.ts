import { metrics } from '@opentelemetry/api';
import { AggregationTemporality, InMemoryMetricExporter, MeterProvider, PeriodicExportingMetricReader } from '@opentelemetry/sdk-metrics';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { _resetMetricsInstrumentCacheForTests, getMetricsClient } from '../index';

/**
 * Drive the metrics module with an in-memory exporter so we can inspect
 * exactly what got recorded without standing up an OTel collector.
 */
const exporter = new InMemoryMetricExporter(AggregationTemporality.DELTA);
const reader = new PeriodicExportingMetricReader({ exporter, exportIntervalMillis: 50 });
const provider = new MeterProvider({ readers: [reader] });
metrics.setGlobalMeterProvider(provider);

async function collect(): Promise<ReturnType<InMemoryMetricExporter['getMetrics']>> {
    await provider.forceFlush();
    return exporter.getMetrics();
}

describe('MetricsClient', () => {
    beforeEach(() => {
        _resetMetricsInstrumentCacheForTests();
        exporter.reset();
    });

    afterEach(() => {
        exporter.reset();
    });

    it('counter increments a Counter instrument with attributes', async () => {
        const m = getMetricsClient('test-service');
        m.counter('agent.turn.completed', 1, { channel: 'voice' });
        m.counter('agent.turn.completed', 1, { channel: 'voice' });
        m.counter('agent.turn.completed', 1, { channel: 'webchat' });

        const recorded = await collect();
        const allPoints = recorded.flatMap((r) => r.scopeMetrics.flatMap((s) => s.metrics));
        const counter = allPoints.find((p) => p.descriptor.name === 'agent.turn.completed');
        expect(counter).toBeDefined();
        const sum = counter!.dataPoints.reduce((acc, dp) => acc + (dp.value as number), 0);
        expect(sum).toBe(3);
    });

    it('histogram records observations', async () => {
        const m = getMetricsClient('test-service');
        m.histogram('agent.tokens.used', 120, { model: 'sonnet' });
        m.histogram('agent.tokens.used', 230, { model: 'sonnet' });

        const recorded = await collect();
        const allPoints = recorded.flatMap((r) => r.scopeMetrics.flatMap((s) => s.metrics));
        const hist = allPoints.find((p) => p.descriptor.name === 'agent.tokens.used');
        expect(hist).toBeDefined();
        expect(hist!.dataPoints.length).toBeGreaterThan(0);
    });

    it('timing emits a histogram with unit=ms', async () => {
        const m = getMetricsClient('test-service');
        m.timing('agent.ttft.ms', 312, { model: 'sonnet' });

        const recorded = await collect();
        const allPoints = recorded.flatMap((r) => r.scopeMetrics.flatMap((s) => s.metrics));
        const hist = allPoints.find((p) => p.descriptor.name === 'agent.ttft.ms');
        expect(hist).toBeDefined();
        expect(hist!.descriptor.unit).toBe('ms');
    });

    it('startTimer records elapsed ms when the stop callback is invoked', async () => {
        const m = getMetricsClient('test-service');
        const stop = m.startTimer('agent.tool.latency.ms', { tool: 'knowledge_search' });
        await new Promise((r) => setTimeout(r, 30));
        stop();

        const recorded = await collect();
        const allPoints = recorded.flatMap((r) => r.scopeMetrics.flatMap((s) => s.metrics));
        const hist = allPoints.find((p) => p.descriptor.name === 'agent.tool.latency.ms');
        expect(hist).toBeDefined();
        // We can't easily inspect histogram bucket values; presence is enough.
        expect(hist!.dataPoints.length).toBeGreaterThan(0);
    });

    it('withTiming tags the recording with status=success on resolve', async () => {
        const m = getMetricsClient('test-service');
        const result = await m.withTiming('agent.turn.duration.ms', async () => {
            await new Promise((r) => setTimeout(r, 10));
            return 42;
        });
        expect(result).toBe(42);

        const recorded = await collect();
        const allPoints = recorded.flatMap((r) => r.scopeMetrics.flatMap((s) => s.metrics));
        const hist = allPoints.find((p) => p.descriptor.name === 'agent.turn.duration.ms');
        expect(hist).toBeDefined();
        const dp = hist!.dataPoints[0]!;
        expect((dp.attributes as Record<string, unknown>).status).toBe('success');
    });

    it('withTiming tags the recording with status=error on throw and rethrows', async () => {
        const m = getMetricsClient('test-service');
        await expect(
            m.withTiming('agent.turn.duration.ms', async () => {
                throw new Error('boom');
            }),
        ).rejects.toThrow('boom');

        const recorded = await collect();
        const allPoints = recorded.flatMap((r) => r.scopeMetrics.flatMap((s) => s.metrics));
        const hist = allPoints.find((p) => p.descriptor.name === 'agent.turn.duration.ms');
        expect(hist).toBeDefined();
        const errorPoint = hist!.dataPoints.find((dp) => (dp.attributes as Record<string, unknown>).status === 'error');
        expect(errorPoint).toBeDefined();
    });

    it('reuses the same instrument across calls (no leaks)', async () => {
        const m = getMetricsClient('test-service');
        for (let i = 0; i < 100; i++) m.counter('agent.spin');

        const recorded = await collect();
        const allPoints = recorded.flatMap((r) => r.scopeMetrics.flatMap((s) => s.metrics));
        const counter = allPoints.find((p) => p.descriptor.name === 'agent.spin');
        expect(counter).toBeDefined();
        const sum = counter!.dataPoints.reduce((acc, dp) => acc + (dp.value as number), 0);
        expect(sum).toBe(100);
    });

    it('all methods swallow internal errors — observability never throws into user code', () => {
        // Force the global MeterProvider to a thrower to simulate a broken setup.
        const originalProvider = metrics.getMeterProvider();
        metrics.setGlobalMeterProvider({
            getMeter() {
                return {
                    createCounter() {
                        throw new Error('synthetic');
                    },
                    createHistogram() {
                        throw new Error('synthetic');
                    },
                } as never;
            },
        } as never);
        try {
            _resetMetricsInstrumentCacheForTests();
            const m = getMetricsClient('broken');
            expect(() => m.counter('x', 1)).not.toThrow();
            expect(() => m.histogram('y', 1)).not.toThrow();
            expect(() => m.timing('z', 1)).not.toThrow();
            const stop = m.startTimer('q');
            expect(() => stop()).not.toThrow();
        } finally {
            metrics.setGlobalMeterProvider(originalProvider);
            _resetMetricsInstrumentCacheForTests();
        }
    });
});
