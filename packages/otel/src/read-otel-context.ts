/**
 * Read the active OpenTelemetry span context into the Smoo correlation shape.
 *
 * Used by the @smooai/observability core Client to stamp `traceId` / `spanId`
 * on every event without taking a hard dependency on `@opentelemetry/api`
 * (the bridge is opt-in via `bridgeClientToOtel()`).
 *
 * Falls back gracefully when no OTel span is active — returns an empty object
 * rather than throwing, so the Smoo SDK stays usable in non-OTel environments.
 */

import { trace } from '@opentelemetry/api';

export interface OtelCorrelation {
    /** 16-byte W3C trace id (32 hex chars). */
    traceId?: string;
    /** 8-byte W3C span id (16 hex chars). */
    spanId?: string;
    /** Whether the current trace is recording (sampled-in). */
    sampled?: boolean;
}

export function readOtelCorrelation(): OtelCorrelation {
    const span = trace.getActiveSpan();
    if (!span) return {};
    const ctx = span.spanContext();
    if (!ctx) return {};
    return {
        traceId: ctx.traceId,
        spanId: ctx.spanId,
        sampled: (ctx.traceFlags & 0x01) === 0x01,
    };
}
