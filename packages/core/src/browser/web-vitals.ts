/**
 * Core Web Vitals RUM one-liner for the browser.
 *
 * Wires the `web-vitals` library's field-data callbacks into a Smoo
 * `MetricsClient` so a customer gets LCP/CLS/INP/FCP/TTFB recorded as
 * metrics with a single call:
 *
 *   ```ts
 *   import { getMetricsClient } from '@smooai/observability/metrics';
 *   import { installWebVitals } from '@smooai/observability/browser';
 *
 *   installWebVitals(getMetricsClient('my-web-app'));
 *   ```
 *
 * PINNED metric contract (must match the apps/web dogfood):
 *   - web.vitals.lcp / .fcp / .inp / .ttfb — histogram, unit `ms`
 *   - web.vitals.cls                       — histogram, unitless raw CLS
 *   - attributes on every point: route, rating, navigation_type
 */
import type { MetricsClient } from '../metrics';

/** Minimal shape of a `web-vitals` Metric — only the fields we read. */
interface WebVitalMetric {
    name: 'CLS' | 'FCP' | 'INP' | 'LCP' | 'TTFB';
    value: number;
    rating: 'good' | 'needs-improvement' | 'poor';
    navigationType: string;
}

// web-vitals metric name → our `ms` metric name. CLS is handled separately
// (unitless float) so it's intentionally absent here.
const MS_VITAL_NAMES: Record<string, string> = {
    LCP: 'web.vitals.lcp',
    FCP: 'web.vitals.fcp',
    INP: 'web.vitals.inp',
    TTFB: 'web.vitals.ttfb',
};

export interface InstallWebVitalsOptions {
    /**
     * Resolve the `route` attribute at report time. Defaults to
     * `location.pathname`. Override for SPA routers that rewrite the path
     * client-side (e.g. return your router's current route).
     */
    route?: () => string;
}

/**
 * Map one web-vitals Metric onto the pinned metric contract and record it.
 * Exported so tests can drive it with a synthetic metric without a browser.
 */
export function recordWebVital(metrics: MetricsClient, metric: WebVitalMetric, route: string): void {
    const attrs = {
        route,
        rating: metric.rating,
        navigation_type: metric.navigationType,
    };
    if (metric.name === 'CLS') {
        // Unitless raw CLS — plain histogram, not a `ms` timing.
        metrics.histogram('web.vitals.cls', metric.value, attrs);
    } else {
        // Safe: the CLS branch above is the only key absent from MS_VITAL_NAMES.
        metrics.timing(MS_VITAL_NAMES[metric.name]!, metric.value, attrs);
    }
}

let installed = false;

/**
 * Start recording Core Web Vitals into `metrics`. Browser-only and
 * idempotent — safe to call unconditionally on every page load; a no-op
 * during SSR and on repeat calls.
 */
export function installWebVitals(metrics: MetricsClient, opts: InstallWebVitalsOptions = {}): void {
    if (typeof window === 'undefined') return; // ponytail: SSR / no-DOM no-op
    if (installed) return;
    installed = true;

    const route = opts.route ?? (() => window.location.pathname);
    const report = (metric: WebVitalMetric) => recordWebVital(metrics, metric, route());

    // Lazy import so web-vitals only loads (and its PerformanceObserver only
    // registers) once a consumer opts in.
    void import('web-vitals').then(({ onLCP, onCLS, onINP, onFCP, onTTFB }) => {
        onLCP(report);
        onCLS(report);
        onINP(report);
        onFCP(report);
        onTTFB(report);
    });
}

/** Test seam — reset the idempotency guard between cases. */
export function _resetWebVitalsInstalledForTests(): void {
    installed = false;
}
