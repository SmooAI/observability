/**
 * @smooai/observability/metrics — OpenTelemetry Meter wrapper for Smoo
 * application metrics.
 *
 * Thin Smoo-flavored API on top of `@opentelemetry/api`'s metrics surface.
 * Same shape as the rest of `@smooai/observability` (`Client.captureException`,
 * `setupOtelSdk`, etc.): a tiny ergonomic layer over OTel so consumers don't
 * have to learn the OTel API just to emit counters.
 *
 * Usage in node services:
 *
 *   ```ts
 *   import { setupOtelSdk } from '@smooai/observability/otel';
 *   import { getMetricsClient } from '@smooai/observability/metrics';
 *
 *   setupOtelSdk({ serviceName: 'smooai-voice' }); // also wires metrics export
 *   const metrics = getMetricsClient('smooai-voice');
 *
 *   metrics.counter('agent.turn.completed', 1, { channel: 'voice', tier: 'pro' });
 *   metrics.timing('agent.ttft.ms', 312, { model: 'sonnet' });
 *   const stop = metrics.startTimer('agent.tool.latency.ms', { tool: 'knowledge_search' });
 *   await doWork();
 *   stop();
 *   ```
 *
 * The same instrument name is reused across calls — instruments are cached
 * by `(meterName, instrumentName)` so we don't leak Meter handles.
 */

import { type Attributes, type Counter, type Histogram, metrics as otelMetrics } from '@opentelemetry/api';

export interface MetricsClient {
    /** Add to a monotonically-increasing counter. Most common shape. */
    counter(name: string, value?: number, attrs?: Record<string, string>): void;
    /**
     * Record a histogram observation. Use this for distributions
     * (latencies, sizes, etc.) — the backend will compute percentiles.
     */
    histogram(name: string, value: number, attrs?: Record<string, string>): void;
    /**
     * Alias for `histogram` with `unit: 'ms'` baked in. Renders nicer in
     * dashboards as a duration.
     */
    timing(name: string, ms: number, attrs?: Record<string, string>): void;
    /**
     * Start a wall-clock timer. Call the returned function when the
     * operation completes to record the elapsed milliseconds as a timing
     * histogram. Use for code that doesn't fit a single async block.
     */
    startTimer(name: string, attrs?: Record<string, string>): () => void;
    /**
     * Wrap an async function in a timing measurement. Records the elapsed
     * ms on success or failure (with `status=success|error` attribute).
     */
    withTiming<T>(name: string, fn: () => Promise<T>, attrs?: Record<string, string>): Promise<T>;
}

const counterCache = new Map<string, Counter>();
const histogramCache = new Map<string, Histogram>();

function getCounter(meterName: string, name: string): Counter {
    const key = `${meterName}::${name}`;
    let inst = counterCache.get(key);
    if (!inst) {
        inst = otelMetrics.getMeter(meterName).createCounter(name);
        counterCache.set(key, inst);
    }
    return inst;
}

function getHistogram(meterName: string, name: string, unit?: string): Histogram {
    const key = `${meterName}::${name}::${unit ?? ''}`;
    let inst = histogramCache.get(key);
    if (!inst) {
        inst = otelMetrics.getMeter(meterName).createHistogram(name, { unit });
        histogramCache.set(key, inst);
    }
    return inst;
}

function toAttributes(attrs?: Record<string, string>): Attributes | undefined {
    if (!attrs) return undefined;
    return attrs as Attributes;
}

/**
 * Build a metrics client bound to a specific service-named meter. Cheap;
 * call per service / module if you want logical grouping.
 *
 * Defaults to meter name `@smooai/observability` so any caller can `getMetricsClient()`
 * with no args and still emit. Production callers pass their service name
 * (e.g. `smooai-voice`, `smooai-backend`) so dashboards can filter by
 * `instrumentation.scope.name`.
 */
export function getMetricsClient(meterName: string = '@smooai/observability'): MetricsClient {
    return {
        counter(name, value = 1, attrs) {
            try {
                getCounter(meterName, name).add(value, toAttributes(attrs));
            } catch {
                /* observability MUST NOT throw into user code */
            }
        },
        histogram(name, value, attrs) {
            try {
                getHistogram(meterName, name).record(value, toAttributes(attrs));
            } catch {
                /* swallow */
            }
        },
        timing(name, ms, attrs) {
            try {
                getHistogram(meterName, name, 'ms').record(ms, toAttributes(attrs));
            } catch {
                /* swallow */
            }
        },
        startTimer(name, attrs) {
            const start = Date.now();
            return () => {
                const ms = Date.now() - start;
                try {
                    getHistogram(meterName, name, 'ms').record(ms, toAttributes(attrs));
                } catch {
                    /* swallow */
                }
            };
        },
        async withTiming(name, fn, attrs) {
            const start = Date.now();
            try {
                const result = await fn();
                const ms = Date.now() - start;
                try {
                    getHistogram(meterName, name, 'ms').record(ms, toAttributes({ ...attrs, status: 'success' }));
                } catch {
                    /* swallow */
                }
                return result;
            } catch (err) {
                const ms = Date.now() - start;
                try {
                    getHistogram(meterName, name, 'ms').record(ms, toAttributes({ ...attrs, status: 'error' }));
                } catch {
                    /* swallow */
                }
                throw err;
            }
        },
    };
}

/** Test seam — drop cached instruments so a fresh MeterProvider takes effect. */
export function _resetMetricsInstrumentCacheForTests(): void {
    counterCache.clear();
    histogramCache.clear();
}
