import { SpanKind, SpanStatusCode, TraceFlags } from '@opentelemetry/api';
import { ExportResultCode } from '@opentelemetry/core';
import { Resource } from '@opentelemetry/resources';
import type { ReadableSpan } from '@opentelemetry/sdk-trace-base';
import { describe, expect, it, vi } from 'vitest';
import type { TokenProvider } from '../../auth/token-provider';
import { AuthInjectingTraceExporter } from '../auth-injecting-exporter';

// A serializable ReadableSpan stub — JsonTraceSerializer reads name, span
// context, kind, timing, attributes, status, resource, and scope, so we
// supply all of them. The exporter then attempts a real export call.
const fakeSpan = {
    name: 'test-span',
    kind: SpanKind.INTERNAL,
    spanContext: () => ({ traceId: '0af7651916cd43dd8448eb211c80319c', spanId: 'b7ad6b7169203331', traceFlags: TraceFlags.SAMPLED }),
    parentSpanId: undefined,
    startTime: [1609459200, 0] as [number, number],
    endTime: [1609459200, 1_000_000] as [number, number],
    status: { code: SpanStatusCode.OK },
    attributes: {},
    links: [],
    events: [],
    duration: [0, 1_000_000] as [number, number],
    ended: true,
    resource: Resource.empty(),
    instrumentationLibrary: { name: 'test', version: '0.0.0' },
    droppedAttributesCount: 0,
    droppedEventsCount: 0,
    droppedLinksCount: 0,
} as unknown as ReadableSpan;

function tokenProviderStub(): TokenProvider & { invalidate: ReturnType<typeof vi.fn> } {
    const invalidate = vi.fn();
    return {
        getAccessToken: vi.fn().mockResolvedValue('tok-123'),
        invalidate,
    } as unknown as TokenProvider & { invalidate: ReturnType<typeof vi.fn> };
}

describe('AuthInjectingTraceExporter (transport via @smooai/fetch seam)', () => {
    it('posts with a fresh Bearer token and reports SUCCESS on 2xx', async () => {
        const fetcher = vi.fn().mockResolvedValue(new Response('', { status: 200 }));
        const tp = tokenProviderStub();
        const exporter = new AuthInjectingTraceExporter({ url: 'https://api.smoo.ai/v1/traces', tokenProvider: tp, fetcher });

        const result = await new Promise<{ code: ExportResultCode }>((resolve) => {
            exporter.export([fakeSpan], resolve);
        });

        expect(result.code).toBe(ExportResultCode.SUCCESS);
        expect(fetcher).toHaveBeenCalledOnce();
        const [url, init] = fetcher.mock.calls[0]!;
        expect(url).toBe('https://api.smoo.ai/v1/traces');
        expect(init.method).toBe('POST');
        expect((init.headers as Record<string, string>).authorization).toBe('Bearer tok-123');
        // Timeout is handed to @smooai/fetch via its `options` bag, not a
        // hand-rolled AbortController.
        expect((init as { options?: { timeout?: { timeoutMs: number } } }).options?.timeout?.timeoutMs).toBe(10_000);
        expect((init as { signal?: unknown }).signal).toBeUndefined();
    });

    it('invalidates the token and retries once on 401, then succeeds', async () => {
        const fetcher = vi
            .fn()
            .mockResolvedValueOnce(new Response('', { status: 401 }))
            .mockResolvedValueOnce(new Response('', { status: 200 }));
        const tp = tokenProviderStub();
        const exporter = new AuthInjectingTraceExporter({ url: 'https://api.smoo.ai/v1/traces', tokenProvider: tp, fetcher });

        const result = await new Promise<{ code: ExportResultCode }>((resolve) => {
            exporter.export([fakeSpan], resolve);
        });

        expect(result.code).toBe(ExportResultCode.SUCCESS);
        expect(tp.invalidate).toHaveBeenCalledOnce();
        expect(fetcher).toHaveBeenCalledTimes(2);
    });

    it('reports FAILED when the response is non-ok (e.g. unwrapped from a thrown HTTPResponseError)', async () => {
        // Simulates the default transport handing back the Response carried on
        // a thrown @smooai/fetch HTTPResponseError after retries are exhausted.
        const fetcher = vi.fn().mockResolvedValue(new Response('boom', { status: 503 }));
        const tp = tokenProviderStub();
        const exporter = new AuthInjectingTraceExporter({ url: 'https://api.smoo.ai/v1/traces', tokenProvider: tp, fetcher });

        const result = await new Promise<{ code: ExportResultCode; error?: Error }>((resolve) => {
            exporter.export([fakeSpan], resolve);
        });

        expect(result.code).toBe(ExportResultCode.FAILED);
        expect(result.error?.message).toContain('503');
    });

    it('reports FAILED when the transport throws (network / circuit-open)', async () => {
        const fetcher = vi.fn().mockRejectedValue(new Error('circuit open'));
        const tp = tokenProviderStub();
        const exporter = new AuthInjectingTraceExporter({ url: 'https://api.smoo.ai/v1/traces', tokenProvider: tp, fetcher });

        const result = await new Promise<{ code: ExportResultCode; error?: Error }>((resolve) => {
            exporter.export([fakeSpan], resolve);
        });

        expect(result.code).toBe(ExportResultCode.FAILED);
        expect(result.error?.message).toContain('circuit open');
    });
});
