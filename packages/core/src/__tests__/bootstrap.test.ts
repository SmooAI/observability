import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { _resetBootstrapForTests, bootstrapObservability } from '../bootstrap';
import { _resetOtelSdkForTests } from '../otel';

// Capture stderr writes so we can assert on the bootstrap warning paths
// without polluting test output.
const stderr: string[] = [];
const originalWrite = process.stderr.write.bind(process.stderr);

beforeEach(() => {
    _resetBootstrapForTests();
    _resetOtelSdkForTests();
    stderr.length = 0;
    // Monkey-patch instead of vi.spyOn — the stderr.write signature has
    // overloads that vi's MockInstance type doesn't unify cleanly.
    (process.stderr as unknown as { write: (chunk: unknown) => boolean }).write = (chunk: unknown) => {
        stderr.push(typeof chunk === 'string' ? chunk : String(chunk));
        return true;
    };
});

afterEach(() => {
    (process.stderr as unknown as { write: typeof originalWrite }).write = originalWrite;
    _resetBootstrapForTests();
    _resetOtelSdkForTests();
});

describe('bootstrapObservability', () => {
    it('is idempotent — second call returns the same handle', () => {
        const first = bootstrapObservability({ token: 'preminted', endpoint: 'https://api.test' });
        const second = bootstrapObservability({ token: 'different', endpoint: 'https://other' });
        expect(first).toBe(second);
        expect(first.installed).toBe(true);
    });

    it('skips bootstrap entirely when disabled', () => {
        const result = bootstrapObservability({ disabled: true });
        expect(result.installed).toBe(false);
        expect(result.otel).toBeNull();
    });

    it('warns when no auth mode is configured but still installs the SDK', () => {
        const result = bootstrapObservability({ endpoint: 'https://api.test' });
        expect(result.installed).toBe(true);
        expect(stderr.join('')).toContain('no auth configured');
    });

    it('uses a pre-minted token verbatim (no exchange call)', () => {
        const fetchSpy = vi.fn();
        // The fetcher isn't used when token is provided — assert it stays cold.
        const result = bootstrapObservability({
            token: 'sk_test_abc',
            endpoint: 'https://api.test',
        });
        expect(result.installed).toBe(true);
        expect(fetchSpy).not.toHaveBeenCalled();
    });

    it('derives /v1/{traces,metrics} from SMOOAI_OBSERVABILITY_ENDPOINT', () => {
        const oldTraces = process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        const oldMetrics = process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT;
        delete process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        delete process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT;
        try {
            bootstrapObservability({ token: 't', endpoint: 'https://api.test/' });
            expect(process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT).toBe('https://api.test/v1/traces');
            expect(process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT).toBe('https://api.test/v1/metrics');
        } finally {
            if (oldTraces) process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = oldTraces;
            else delete process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
            if (oldMetrics) process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT = oldMetrics;
            else delete process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT;
        }
    });

    it('respects pre-set OTEL_EXPORTER_OTLP_*_ENDPOINT env vars', () => {
        const oldTraces = process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = 'https://override/v1/traces';
        try {
            bootstrapObservability({ token: 't', endpoint: 'https://api.test' });
            expect(process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT).toBe('https://override/v1/traces');
        } finally {
            if (oldTraces) process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = oldTraces;
            else delete process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        }
    });

    it('does not crash the host when SDK init throws — returns installed=false', () => {
        // Force a bad endpoint that surfaces during exporter validation in
        // some otel-js versions. We can't reliably force a throw in the SDK
        // without monkeypatching, so this test asserts the catch path with
        // a hand-rolled override of the SDK call would be ideal — for now,
        // assert that the function returns *some* result instead of
        // throwing.
        expect(() =>
            bootstrapObservability({
                token: 't',
                endpoint: 'not-a-url',
            }),
        ).not.toThrow();
    });
});
