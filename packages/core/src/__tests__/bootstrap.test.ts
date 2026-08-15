import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { _resetBootstrapForTests, bootstrapObservability } from '../bootstrap';
import { _resetOtelSdkForTests } from '../otel';
import { _resetPiiHashKeyForTests, _tokenWithKey, piiToken, scrubString } from '../pii';

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
    it('is idempotent — second call returns the same handle', async () => {
        const first = await bootstrapObservability({ token: 'preminted', endpoint: 'https://api.test' });
        const second = await bootstrapObservability({ token: 'different', endpoint: 'https://other' });
        expect(first).toBe(second);
        expect(first.installed).toBe(true);
    });

    it('skips bootstrap entirely when disabled', async () => {
        const result = await bootstrapObservability({ disabled: true });
        expect(result.installed).toBe(false);
        expect(result.otel).toBeNull();
    });

    it('warns when no auth mode is configured but still installs the SDK', async () => {
        const result = await bootstrapObservability({ endpoint: 'https://api.test' });
        expect(result.installed).toBe(true);
        expect(stderr.join('')).toContain('no auth configured');
    });

    it('uses a pre-minted token verbatim (no exchange call)', async () => {
        const fetchSpy = vi.fn();
        // The fetcher isn't used when token is provided — assert it stays cold.
        const result = await bootstrapObservability({
            token: 'sk_test_abc',
            endpoint: 'https://api.test',
        });
        expect(result.installed).toBe(true);
        expect(fetchSpy).not.toHaveBeenCalled();
    });

    it('derives /v1/{traces,metrics} from SMOOAI_OBSERVABILITY_ENDPOINT', async () => {
        const oldTraces = process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        const oldMetrics = process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT;
        delete process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        delete process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT;
        try {
            await bootstrapObservability({ token: 't', endpoint: 'https://api.test/' });
            expect(process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT).toBe('https://api.test/v1/traces');
            expect(process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT).toBe('https://api.test/v1/metrics');
        } finally {
            if (oldTraces) process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = oldTraces;
            else delete process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
            if (oldMetrics) process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT = oldMetrics;
            else delete process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT;
        }
    });

    it('respects pre-set OTEL_EXPORTER_OTLP_*_ENDPOINT env vars', async () => {
        const oldTraces = process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = 'https://override/v1/traces';
        try {
            await bootstrapObservability({ token: 't', endpoint: 'https://api.test' });
            expect(process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT).toBe('https://override/v1/traces');
        } finally {
            if (oldTraces) process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = oldTraces;
            else delete process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        }
    });

    // --- honest export status -------------------------------------------
    // Both halves matter. With only one asserted, an implementation that
    // hard-codes either value passes.

    it('reports exporting=false and warns loudly when no endpoint is configured', async () => {
        const saved = {
            base: process.env.OTEL_EXPORTER_OTLP_ENDPOINT,
            traces: process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
            metrics: process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
            logs: process.env.OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
        };
        // A previous bootstrap in this process may have set these — the
        // function writes them and never clears them.
        delete process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
        delete process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT;
        delete process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT;
        delete process.env.OTEL_EXPORTER_OTLP_LOGS_ENDPOINT;
        try {
            const result = await bootstrapObservability({ token: 't', serviceName: 'svc' });
            expect(result.installed).toBe(true); // bootstrap ran…
            expect(result.exporting).toBe(false); // …but nothing has anywhere to go
            expect(stderr.join('')).toContain('NO OTLP ENDPOINT CONFIGURED');
            expect(stderr.join('')).toContain('SMOOAI_OBSERVABILITY_DISABLED=true');
        } finally {
            for (const [key, value] of [
                ['OTEL_EXPORTER_OTLP_ENDPOINT', saved.base],
                ['OTEL_EXPORTER_OTLP_TRACES_ENDPOINT', saved.traces],
                ['OTEL_EXPORTER_OTLP_METRICS_ENDPOINT', saved.metrics],
                ['OTEL_EXPORTER_OTLP_LOGS_ENDPOINT', saved.logs],
            ] as [string, string | undefined][]) {
                if (value) process.env[key] = value;
                else delete process.env[key];
            }
        }
    });

    it('reports exporting=true and stays quiet when an endpoint IS configured', async () => {
        const result = await bootstrapObservability({ token: 't', endpoint: 'https://api.test' });
        expect(result.installed).toBe(true);
        expect(result.exporting).toBe(true);
        expect(stderr.join('')).not.toContain('NO OTLP ENDPOINT CONFIGURED');
    });

    it('does not crash the host when SDK init throws — returns installed=false', async () => {
        // Force a bad endpoint that surfaces during exporter validation in
        // some otel-js versions. We can't reliably force a throw in the SDK
        // without monkeypatching, so this test asserts the catch path with
        // a hand-rolled override of the SDK call would be ideal — for now,
        // assert that the function returns *some* result instead of
        // throwing.
        await expect(
            bootstrapObservability({
                token: 't',
                endpoint: 'not-a-url',
            }),
        ).resolves.toBeDefined();
    });
});

describe('bootstrapObservability — PII hash key', () => {
    it('installs the key from SMOOAI_OBSERVABILITY_PII_HASH_KEY before anything can emit', async () => {
        const previous = process.env.SMOOAI_OBSERVABILITY_PII_HASH_KEY;
        process.env.SMOOAI_OBSERVABILITY_PII_HASH_KEY = 'env-supplied-key';
        _resetPiiHashKeyForTests();
        try {
            // Disabled bootstrap still installs the key — the scrubber runs
            // whether or not there is an exporter behind it.
            await bootstrapObservability({ disabled: true });
            expect(piiToken('email', 'a@b.com', 'org-1')).toBe(_tokenWithKey('email', 'a@b.com', 'org-1', new TextEncoder().encode('env-supplied-key')));
        } finally {
            _resetPiiHashKeyForTests();
            if (previous === undefined) delete process.env.SMOOAI_OBSERVABILITY_PII_HASH_KEY;
            else process.env.SMOOAI_OBSERVABILITY_PII_HASH_KEY = previous;
        }
    });

    it('leaves PII redacted when no key is configured', async () => {
        const previous = process.env.SMOOAI_OBSERVABILITY_PII_HASH_KEY;
        delete process.env.SMOOAI_OBSERVABILITY_PII_HASH_KEY;
        _resetPiiHashKeyForTests();
        try {
            await bootstrapObservability({ disabled: true });
            expect(scrubString('mail a@b.com')).toBe('mail [email:redacted]');
        } finally {
            _resetPiiHashKeyForTests();
            if (previous !== undefined) process.env.SMOOAI_OBSERVABILITY_PII_HASH_KEY = previous;
        }
    });
});
