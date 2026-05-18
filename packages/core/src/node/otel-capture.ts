/**
 * OTel-native capture handler for the node runtime.
 *
 * Registered as the Client's CaptureHandler from `node/index.ts`. Replaces
 * the Smoo-native HTTP transport on Node — every captured exception /
 * message becomes a span event on the active OpenTelemetry span (or a
 * synthetic one if no span is active), with `SpanStatusCode.ERROR` for
 * exceptions and OTLP-shaped attributes for tags / user / release. The
 * OTel SDK handles batching, retry, and wire format; the Smoo SDK doesn't
 * need its own HTTP pipeline on Node.
 *
 * Browser stays on the Smoo-native transport (OTel browser SDK is too
 * heavy for customer-facing bundles). The Client itself doesn't know about
 * either runtime — the runtime entry wires the appropriate path.
 */

import { type Attributes, SpanStatusCode, trace } from '@opentelemetry/api';
import type { CaptureHandler, _Client } from '../client';
import { Client } from '../client';
import type { ObservabilityEvent } from '../types';

interface RegisterOptions {
    /** OTel tracer name. Defaults to 'smooai.observability'. */
    tracerName?: string;
}

let installed = false;

export function registerOtelCapture(opts: RegisterOptions = {}): void {
    if (installed) return;
    installed = true;

    const tracer = trace.getTracer(opts.tracerName ?? 'smooai.observability');

    const handler: CaptureHandler = (event, raw) => {
        const active = trace.getActiveSpan();
        if (active) {
            recordOnSpan(active, event, raw);
            return;
        }
        // No active span — mint a synthetic one so the error still surfaces
        // in the trace. Background workers (cron, queue consumers) hit this
        // path when they capture errors outside any HTTP request context.
        const span = tracer.startSpan(event.exception?.length ? 'observability.captureException' : 'observability.captureMessage');
        try {
            recordOnSpan(span, event, raw);
        } finally {
            span.end();
        }
    };

    (Client as unknown as _Client)._registerCaptureHandler(handler);
}

/** Test seam — un-register so the next call re-installs cleanly. */
export function _resetOtelCaptureForTests(): void {
    installed = false;
    (Client as unknown as _Client)._registerCaptureHandler(null);
}

function recordOnSpan(
    span: NonNullable<ReturnType<typeof trace.getActiveSpan>>,
    event: ObservabilityEvent,
    raw: { error?: unknown; message?: string; extra?: { tags?: Record<string, string> } },
): void {
    const isException = (event.exception?.length ?? 0) > 0;
    const attrs: Attributes = {
        'smoo.event_id': event.eventId,
        ...(event.environment ? { 'deployment.environment.name': event.environment } : {}),
        ...(event.release ? { 'service.version': event.release } : {}),
        ...(event.level ? { 'smoo.level': event.level } : {}),
    };
    if (event.user?.id) attrs['enduser.id'] = event.user.id;
    if (event.user?.orgId) attrs['enduser.org_id'] = event.user.orgId;
    if (event.user?.sessionId) attrs['enduser.session_id'] = event.user.sessionId;
    if (event.tags) {
        for (const [k, v] of Object.entries(event.tags)) {
            attrs[`smoo.tag.${k}`] = v;
        }
    }

    if (isException) {
        const err = raw.error;
        if (err instanceof Error) {
            span.recordException(err);
        } else {
            span.recordException(new Error(typeof err === 'string' ? err : 'non-Error captured'));
        }
        span.setStatus({
            code: SpanStatusCode.ERROR,
            message: err instanceof Error ? err.message : event.exception?.[0]?.value,
        });
    } else if (event.message) {
        // captureMessage path — emit as a span event with the message; status
        // stays UNSET unless the level is 'error' / 'fatal'.
        span.addEvent('smoo.message', { ...attrs, 'smoo.message': event.message });
        if (event.level === 'error' || event.level === 'fatal') {
            span.setStatus({ code: SpanStatusCode.ERROR, message: event.message });
        }
    }
    if (Object.keys(attrs).length > 0) span.setAttributes(attrs);
}
