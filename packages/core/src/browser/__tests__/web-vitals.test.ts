import { describe, expect, it } from 'vitest';
import type { MetricsClient } from '../../metrics';
import { recordWebVital } from '../web-vitals';

/**
 * Drive the pinned web.vitals.* contract with a fake MetricsClient so we can
 * assert the exact metric name / recording kind / attributes for a simulated
 * vital — no real browser or PerformanceObserver needed.
 */
type Recorded = { kind: 'timing' | 'histogram'; name: string; value: number; attrs?: Record<string, string> };

function fakeMetrics(): { client: MetricsClient; recorded: Recorded[] } {
    const recorded: Recorded[] = [];
    const client: MetricsClient = {
        counter: () => {},
        histogram: (name, value, attrs) => recorded.push({ kind: 'histogram', name, value, attrs }),
        timing: (name, ms, attrs) => recorded.push({ kind: 'timing', name, value: ms, attrs }),
        startTimer: () => () => {},
        withTiming: async (_name, fn) => fn(),
    };
    return { client, recorded };
}

const metric = (name: string, value: number) => ({
    name: name as never,
    value,
    rating: 'good' as const,
    navigationType: 'navigate',
});

describe('recordWebVital', () => {
    it('records LCP/FCP/INP/TTFB as `ms` timings under the pinned names', () => {
        const cases: Array<[string, string]> = [
            ['LCP', 'web.vitals.lcp'],
            ['FCP', 'web.vitals.fcp'],
            ['INP', 'web.vitals.inp'],
            ['TTFB', 'web.vitals.ttfb'],
        ];
        for (const [vital, expectedName] of cases) {
            const { client, recorded } = fakeMetrics();
            recordWebVital(client, metric(vital, 1234), '/pricing');
            expect(recorded).toEqual([
                {
                    kind: 'timing',
                    name: expectedName,
                    value: 1234,
                    attrs: { route: '/pricing', rating: 'good', navigation_type: 'navigate' },
                },
            ]);
        }
    });

    it('records CLS as a unitless histogram with the raw value', () => {
        const { client, recorded } = fakeMetrics();
        recordWebVital(client, metric('CLS', 0.042), '/');
        expect(recorded).toEqual([
            {
                kind: 'histogram',
                name: 'web.vitals.cls',
                value: 0.042,
                attrs: { route: '/', rating: 'good', navigation_type: 'navigate' },
            },
        ]);
    });

    it('propagates rating and navigation_type from the metric', () => {
        const { client, recorded } = fakeMetrics();
        recordWebVital(client, { name: 'LCP' as never, value: 5000, rating: 'poor', navigationType: 'back-forward' }, '/slow');
        expect(recorded[0]!.attrs).toEqual({ route: '/slow', rating: 'poor', navigation_type: 'back-forward' });
    });
});
