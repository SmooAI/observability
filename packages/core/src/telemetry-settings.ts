/**
 * ADR-097 W1 — config-served telemetry settings.
 *
 * These are `@smooai/config` **public-tier**, org-scoped keys. Public tier is
 * mandatory, not a convenience: a browser can never be served secret tier
 * (ADR-075 tier boundary), and the whole point is that changing a key changes
 * every client's behaviour on its next config read — no redeploy, no app-store
 * review. Sampling rates are not sensitive; **no secret may ever enter this key
 * set**.
 *
 * This module does NOT import `@smooai/config`. The SDK ships a narrow,
 * injectable provider seam (`TelemetrySettingsProvider`) so that:
 *   - the SDK stays usable in a test, or a browser bundle, with no network;
 *   - the host app supplies whichever config client it already has;
 *   - the five language ports have one small surface to reproduce rather than
 *     five different config-client integrations.
 *
 * FAIL-SAFE IS THE POINT. Unreachable provider, thrown provider, malformed
 * payload, out-of-range value → the compiled-in ADR-010 defaults. Never
 * "sample everything out": a telemetry system that goes silent when its config
 * server hiccups is worse than useless.
 */

import { parseLevel, type CanonicalLevel } from './sampling';

/** The `@smooai/config` public-tier key names this SDK reads. */
export const TELEMETRY_SETTING_KEYS = {
    /** boolean — kill switch. false disables ALL telemetry emission. */
    enabled: 'observabilityEnabled',
    /** number 0.0–1.0 — session-scoped browser log sampling ratio. */
    browserLogSamplingRatio: 'observabilityBrowserLogSamplingRatio',
    /** string — minimum log level to emit (TRACE|DEBUG|INFO|WARN|ERROR|FATAL). */
    minimumLogLevel: 'observabilityMinimumLogLevel',
    /** number 0.0–1.0 — head-based trace sampling ratio. */
    traceSamplingRatio: 'observabilityTraceSamplingRatio',
} as const;

export interface TelemetrySettings {
    /** Kill switch. When false nothing is emitted, errors included. */
    enabled: boolean;
    /**
     * Session-scoped browser log sampling ratio. Applied ONCE per session (or
     * inherited from the trace where one exists) — never per line.
     * Default 1.0: ADR-010 keeps logs unsampled until an operator says otherwise.
     */
    browserLogSamplingRatio: number;
    /** Minimum level to emit, canonical uppercase. */
    minimumLogLevel: CanonicalLevel;
    /** Head-based trace sampling ratio. ADR-010 default: TraceIdRatioBased(0.1). */
    traceSamplingRatio: number;
}

/** Compiled-in ADR-010 defaults. Every failure path lands here. */
export const DEFAULT_TELEMETRY_SETTINGS: Readonly<TelemetrySettings> = Object.freeze({
    enabled: true,
    browserLogSamplingRatio: 1.0,
    minimumLogLevel: 'INFO' as CanonicalLevel,
    traceSamplingRatio: 0.1,
});

/**
 * The seam. Returns a bag of raw config values keyed by
 * `TELEMETRY_SETTING_KEYS` — typically `await publicConfig.get(...)` results,
 * but a plain object in tests. May return null/undefined, may throw, may return
 * garbage; `loadTelemetrySettings` handles all three.
 */
export type TelemetrySettingsProvider = () => unknown | Promise<unknown>;

/**
 * Ratio coercion.
 *
 * - finite number (or numeric string — public config often round-trips as
 *   strings) → clamped into [0, 1]. An operator who writes 1.5 means "all".
 * - anything else (missing, NaN, Infinity, boolean, object, unparseable
 *   string) → the compiled-in default. Never 0.
 *
 * Note the deliberate asymmetry: a *malformed* value falls back to the default,
 * a *valid but out-of-range* value is clamped. -1 clamps to 0 (telemetry off)
 * because that is an explicit operator value, and 0 is settable anyway.
 */
function coerceRatio(raw: unknown, fallback: number): number {
    const n = typeof raw === 'number' ? raw : typeof raw === 'string' && raw.trim() !== '' ? Number(raw) : NaN;
    if (!Number.isFinite(n)) return fallback;
    return Math.min(1, Math.max(0, n));
}

function coerceBoolean(raw: unknown, fallback: boolean): boolean {
    if (typeof raw === 'boolean') return raw;
    if (typeof raw === 'string') {
        const s = raw.trim().toLowerCase();
        if (s === 'true') return true;
        if (s === 'false') return false;
    }
    return fallback;
}

// `parseLevel` (not `normalizeLevel`) on purpose: normalizeLevel maps unknown
// spellings to INFO, which is right for an incoming log line but wrong here — a
// typo'd config value must fall back to the default, not silently reset the floor.
function coerceLevel(raw: unknown, fallback: CanonicalLevel): CanonicalLevel {
    return (typeof raw === 'string' ? parseLevel(raw) : null) ?? fallback;
}

/**
 * Turn a raw config payload into settings. Total function — never throws,
 * always returns a usable object. Unknown/extra keys are ignored.
 */
export function resolveTelemetrySettings(raw: unknown): TelemetrySettings {
    const d = DEFAULT_TELEMETRY_SETTINGS;
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return { ...d };
    const bag = raw as Record<string, unknown>;
    return {
        enabled: coerceBoolean(bag[TELEMETRY_SETTING_KEYS.enabled], d.enabled),
        browserLogSamplingRatio: coerceRatio(bag[TELEMETRY_SETTING_KEYS.browserLogSamplingRatio], d.browserLogSamplingRatio),
        minimumLogLevel: coerceLevel(bag[TELEMETRY_SETTING_KEYS.minimumLogLevel], d.minimumLogLevel),
        traceSamplingRatio: coerceRatio(bag[TELEMETRY_SETTING_KEYS.traceSamplingRatio], d.traceSamplingRatio),
    };
}

/**
 * Read settings through a provider, falling back to defaults on ANY failure
 * (throw, rejection, null, garbage). This is the fail-safe guarantee stated in
 * ADR-097 §2, and it is why this function has no error channel: a caller that
 * could see an error would be tempted to disable telemetry on it.
 */
export async function loadTelemetrySettings(provider?: TelemetrySettingsProvider): Promise<TelemetrySettings> {
    if (!provider) return { ...DEFAULT_TELEMETRY_SETTINGS };
    try {
        return resolveTelemetrySettings(await provider());
    } catch {
        return { ...DEFAULT_TELEMETRY_SETTINGS };
    }
}
