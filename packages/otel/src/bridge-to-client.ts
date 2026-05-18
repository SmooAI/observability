/**
 * Bridge `@smooai/observability` core Client → OpenTelemetry.
 *
 * Effect of `bridgeClientToOtel()`:
 *   1. Every captured exception becomes a recorded exception on the active
 *      OTel span and sets the span status to ERROR. If no span is active,
 *      a synthetic span is created so the error still surfaces in the trace.
 *   2. Every Smoo event picks up `traceId` + `spanId` from the active OTel
 *      context so dashboards correlate one-click between traces and errors.
 *   3. Tag/user updates propagate to span attributes.
 *
 * The bridge wraps `Client.captureException` / `Client.setUser` / `Client.setTag`
 * rather than re-implementing them, so the existing Smoo wire format keeps
 * working — OTel becomes an additional output, not a replacement.
 *
 * Idempotent — installing twice is a no-op.
 *
 * After Phase 2 (ingest swap) and Phase 3 (metrics SDK rebuild on OTel meters),
 * this bridge becomes the primary capture path and the legacy transport
 * becomes optional.
 */

import { type Attributes, SpanStatusCode, trace } from '@opentelemetry/api';
import { Client } from '@smooai/observability';
import { readOtelCorrelation } from './read-otel-context';

let installed = false;
// Save the un-wrapped originals so `_resetBridgeForTests` can fully restore
// them. Without this, calling reset → bridgeClientToOtel again wraps the
// already-wrapped functions, causing double captures in subsequent tests.
interface OriginalRefs {
    capture: ClientLike['captureException'];
    setUser: ClientLike['setUser'];
    setTag: ClientLike['setTag'];
}
let originalRefs: OriginalRefs | null = null;

export interface BridgeOptions {
    /**
     * Tracer name used when we have to mint a synthetic span (no active span
     * at capture time). Default: 'smooai.observability'.
     */
    tracerName?: string;
}

interface ClientLike {
    _isInitialized: () => boolean;
    captureException: (error: unknown, extra?: { tags?: Record<string, string> }) => string | undefined;
    setUser?: (user: { id?: string; orgId?: string; sessionId?: string } | undefined) => void;
    setTag?: (key: string, value: string) => void;
}

export function bridgeClientToOtel(options: BridgeOptions = {}): void {
    if (installed) return;
    installed = true;

    const tracer = trace.getTracer(options.tracerName ?? 'smooai.observability');
    const client = Client as unknown as ClientLike;
    originalRefs = {
        capture: client.captureException.bind(Client),
        setUser: client.setUser?.bind(Client),
        setTag: client.setTag?.bind(Client),
    };
    const originalCapture = originalRefs.capture;
    const originalSetUser = originalRefs.setUser;
    const originalSetTag = originalRefs.setTag;

    client.captureException = (error, extra) => {
        const eventId = originalCapture(error, extra);
        try {
            const active = trace.getActiveSpan();
            if (active) {
                recordOnSpan(active, error, eventId, extra?.tags);
            } else {
                // No active span — mint a one-off so the trace still has signal.
                const span = tracer.startSpan('observability.captureException');
                try {
                    recordOnSpan(span, error, eventId, extra?.tags);
                } finally {
                    span.end();
                }
            }
        } catch {
            /* swallow — bridge must never throw into user code */
        }
        return eventId;
    };

    if (originalSetUser) {
        client.setUser = (user) => {
            originalSetUser(user);
            try {
                const span = trace.getActiveSpan();
                if (span && user) {
                    if (user.id) span.setAttribute('enduser.id', user.id);
                    if (user.orgId) span.setAttribute('enduser.org_id', user.orgId);
                    if (user.sessionId) span.setAttribute('enduser.session_id', user.sessionId);
                }
            } catch {
                /* swallow */
            }
        };
    }

    if (originalSetTag) {
        client.setTag = (key, value) => {
            originalSetTag(key, value);
            try {
                trace.getActiveSpan()?.setAttribute(`smoo.tag.${key}`, value);
            } catch {
                /* swallow */
            }
        };
    }
}

function recordOnSpan(
    span: ReturnType<typeof trace.getActiveSpan> extends infer S ? Exclude<S, undefined> : never,
    error: unknown,
    eventId: string | undefined,
    tags: Record<string, string> | undefined,
): void {
    if (!span) return;
    if (error instanceof Error) {
        span.recordException(error);
    } else {
        span.recordException(new Error(typeof error === 'string' ? error : 'non-Error captured'));
    }
    const attrs: Attributes = {};
    if (eventId) attrs['smoo.event_id'] = eventId;
    if (tags) {
        for (const [k, v] of Object.entries(tags)) {
            attrs[`smoo.tag.${k}`] = v;
        }
    }
    if (Object.keys(attrs).length > 0) span.setAttributes(attrs);
    span.setStatus({ code: SpanStatusCode.ERROR, message: error instanceof Error ? error.message : String(error) });
}

/**
 * Test seam — restore the un-wrapped Client methods so re-bridging in test
 * runs doesn't compound wrappers. No-op outside tests.
 */
export function _resetBridgeForTests(): void {
    if (originalRefs) {
        const client = Client as unknown as ClientLike;
        client.captureException = originalRefs.capture;
        if (originalRefs.setUser) client.setUser = originalRefs.setUser;
        if (originalRefs.setTag) client.setTag = originalRefs.setTag;
        originalRefs = null;
    }
    installed = false;
}

// Re-export so consumers have one import path for everything.
export { readOtelCorrelation } from './read-otel-context';
