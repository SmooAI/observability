/**
 * ADR-097 W1/W2 behaviour tests — the properties the corpus cannot express
 * (published hash vectors, provider failure modes, session stability).
 */
import { describe, expect, it, vi } from 'vitest';
import { createDropCounter, fnv1a32, meetsMinimumLevel, sampleDecision, shouldEmitLog } from '../sampling';
import { DEFAULT_TELEMETRY_SETTINGS, loadTelemetrySettings, TELEMETRY_SETTING_KEYS } from '../telemetry-settings';
import { formatTraceparent, parseTraceparent } from '../traceparent';

describe('fnv1a32', () => {
    // Published FNV-1a-32 test vectors. These are the anchor: if the corpus and
    // the implementation are both wrong in the same way, these still catch it.
    it.each([
        ['', 0x811c9dc5],
        ['a', 0xe40c292c],
        ['foobar', 0xbf9cf968],
    ])('hashes %j to the published vector', (input, expected) => {
        expect(fnv1a32(input)).toBe(expected);
    });

    it('hashes UTF-8 bytes, not UTF-16 code units', () => {
        // '🎉' is one code point, two UTF-16 units, four UTF-8 bytes (f0 9f 8e 89).
        // Fold those four bytes by hand and require the implementation to agree —
        // a port that walks UTF-16 units or code points gets a different answer.
        let expected = 0x811c9dc5;
        for (const b of [0xf0, 0x9f, 0x8e, 0x89]) expected = Math.imul(expected ^ b, 0x01000193) >>> 0;
        expect(fnv1a32('🎉')).toBe(expected);
    });

    it('stays inside unsigned 32-bit for long inputs', () => {
        const h = fnv1a32('x'.repeat(10_000));
        expect(Number.isInteger(h)).toBe(true);
        expect(h).toBeGreaterThanOrEqual(0);
        expect(h).toBeLessThan(2 ** 32);
    });
});

describe('sampleDecision', () => {
    it('is exact at the boundaries for every id', () => {
        for (let i = 0; i < 1000; i++) {
            expect(sampleDecision(`session-${i}`, 1)).toBe(true);
            expect(sampleDecision(`session-${i}`, 0)).toBe(false);
        }
    });

    it('is stable across repeated calls (a page must not flip mid-session)', () => {
        const first = sampleDecision('session-abc', 0.37);
        for (let i = 0; i < 100; i++) expect(sampleDecision('session-abc', 0.37)).toBe(first);
    });

    it('is monotonic in ratio', () => {
        for (const id of ['a', 'b', 'session-xyz', '🎉']) {
            let seenIn = false;
            for (const ratio of [0, 0.1, 0.2, 0.4, 0.6, 0.8, 1]) {
                const decision = sampleDecision(id, ratio);
                if (seenIn) expect(decision).toBe(true);
                seenIn ||= decision;
            }
        }
    });

    it('lands near the requested ratio over a population', () => {
        const n = 20_000;
        let kept = 0;
        for (let i = 0; i < n; i++) if (sampleDecision(`session-${i}`, 0.25)) kept++;
        expect(kept / n).toBeGreaterThan(0.235);
        expect(kept / n).toBeLessThan(0.265);
    });

    it('fails open on non-finite ratios rather than going silently dark', () => {
        expect(sampleDecision('s', Number.NaN)).toBe(true);
        expect(sampleDecision('s', Number.POSITIVE_INFINITY)).toBe(true);
    });
});

describe('shouldEmitLog', () => {
    const base = { enabled: true, minimumLevel: 'INFO' as const, logSamplingRatio: 0, sessionId: 'session-out' };

    it('never drops a warning or error, even at ratio 0', () => {
        for (const level of ['warn', 'warning', 'error', 'fatal', 'critical']) {
            expect(shouldEmitLog({ ...base, level })).toBe(true);
        }
    });

    it('honours the kill switch above everything', () => {
        expect(shouldEmitLog({ ...base, enabled: false, level: 'fatal' })).toBe(false);
    });

    it('inherits the trace decision over the session decision, both ways', () => {
        expect(shouldEmitLog({ ...base, level: 'info', logSamplingRatio: 0, traceSampled: true })).toBe(true);
        expect(shouldEmitLog({ ...base, level: 'info', logSamplingRatio: 1, traceSampled: false })).toBe(false);
    });

    it('keeps ALL lines of a sampled-in session and NONE of a sampled-out one', () => {
        // The ADR's acceptance test: any trace you can open has 100% of its lines.
        for (const sessionId of ['session-a', 'session-b', 'session-out', 'session-zzz']) {
            const decisions = Array.from({ length: 50 }, (_, i) =>
                shouldEmitLog({ ...base, sessionId, level: 'info', logSamplingRatio: 0.5, minimumLevel: 'INFO' }),
            );
            expect(new Set(decisions).size).toBe(1);
        }
    });

    it('applies the minimum level before the always-on rule', () => {
        expect(shouldEmitLog({ ...base, level: 'error', minimumLevel: 'FATAL' })).toBe(false);
        expect(shouldEmitLog({ ...base, level: 'debug', minimumLevel: 'INFO' })).toBe(false);
        expect(shouldEmitLog({ ...base, level: 'debug', minimumLevel: 'DEBUG', logSamplingRatio: 1 })).toBe(true);
    });
});

describe('meetsMinimumLevel', () => {
    it('orders levels', () => {
        expect(meetsMinimumLevel('ERROR', 'WARN')).toBe(true);
        expect(meetsMinimumLevel('WARN', 'WARN')).toBe(true);
        expect(meetsMinimumLevel('INFO', 'WARN')).toBe(false);
    });
});

describe('createDropCounter', () => {
    it('tallies and drains — sampled-out volume stays observable', () => {
        const c = createDropCounter();
        expect(c.drain()).toEqual({});
        c.record('INFO');
        c.record('INFO');
        c.record('DEBUG');
        expect(c.drain()).toEqual({ INFO: 2, DEBUG: 1 });
        expect(c.drain()).toEqual({});
    });
});

describe('traceparent round-trip', () => {
    it('round-trips every valid header', () => {
        const header = '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01';
        const parsed = parseTraceparent(header)!;
        expect(formatTraceparent(parsed)).toBe(header);
    });

    it('refuses to emit a header its own parser would reject', () => {
        expect(formatTraceparent({ traceId: '0'.repeat(32), spanId: '00f067aa0ba902b7', sampled: true })).toBeNull();
    });
});

describe('loadTelemetrySettings — fail-safe', () => {
    it('returns defaults with no provider at all (usable offline / in tests)', async () => {
        await expect(loadTelemetrySettings()).resolves.toEqual(DEFAULT_TELEMETRY_SETTINGS);
    });

    it('returns defaults when the provider throws (config unreachable)', async () => {
        const provider = vi.fn(() => {
            throw new Error('ECONNREFUSED');
        });
        await expect(loadTelemetrySettings(provider)).resolves.toEqual(DEFAULT_TELEMETRY_SETTINGS);
        expect(provider).toHaveBeenCalledOnce();
    });

    it('returns defaults when the provider rejects', async () => {
        await expect(loadTelemetrySettings(() => Promise.reject(new Error('502')))).resolves.toEqual(DEFAULT_TELEMETRY_SETTINGS);
    });

    it('returns defaults on a malformed payload — never "sample everything out"', async () => {
        for (const payload of [null, undefined, '<html>502</html>', 42, []]) {
            const settings = await loadTelemetrySettings(() => payload);
            expect(settings).toEqual(DEFAULT_TELEMETRY_SETTINGS);
            expect(settings.browserLogSamplingRatio).toBe(1);
            expect(settings.enabled).toBe(true);
        }
    });

    it('applies a well-formed payload', async () => {
        await expect(
            loadTelemetrySettings(() => ({
                [TELEMETRY_SETTING_KEYS.enabled]: true,
                [TELEMETRY_SETTING_KEYS.browserLogSamplingRatio]: 0.2,
                [TELEMETRY_SETTING_KEYS.minimumLogLevel]: 'warn',
                [TELEMETRY_SETTING_KEYS.traceSamplingRatio]: 0.05,
            })),
        ).resolves.toEqual({ enabled: true, browserLogSamplingRatio: 0.2, minimumLogLevel: 'WARN', traceSamplingRatio: 0.05 });
    });
});
