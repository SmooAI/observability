import { SeverityNumber } from '@opentelemetry/api-logs';
import { ExportResultCode } from '@opentelemetry/core';
import { Resource } from '@opentelemetry/resources';
import type { ReadableLogRecord } from '@opentelemetry/sdk-logs';
import { describe, expect, it, vi } from 'vitest';
import type { TokenProvider } from '../../auth/token-provider';
import { AuthInjectingLogExporter } from '../auth-injecting-exporter';

// A serializable ReadableLogRecord stub. JsonLogsSerializer reads timing,
// severity, body, attributes, the (optional) span context, resource, and
// scope — supply all of them so a real serialize + export runs. The span
// context here proves a log carrying real W3C ids round-trips through the
// exporter unchanged.
const fakeLog = {
    hrTime: [1609459200, 0] as [number, number],
    hrTimeObserved: [1609459200, 0] as [number, number],
    severityNumber: SeverityNumber.INFO,
    severityText: 'info',
    body: 'hello from a span',
    attributes: { foo: 'bar' },
    droppedAttributesCount: 0,
    spanContext: { traceId: '0af7651916cd43dd8448eb211c80319c', spanId: 'b7ad6b7169203331', traceFlags: 1 },
    resource: Resource.empty(),
    instrumentationScope: { name: '@smooai/logger', version: '0.0.0' },
} as unknown as ReadableLogRecord;

function tokenProviderStub(): TokenProvider & { invalidate: ReturnType<typeof vi.fn> } {
    const invalidate = vi.fn();
    return {
        getAccessToken: vi.fn().mockResolvedValue('tok-123'),
        invalidate,
    } as unknown as TokenProvider & { invalidate: ReturnType<typeof vi.fn> };
}

describe('AuthInjectingLogExporter (transport via @smooai/fetch seam)', () => {
    it('posts the serialized log with a fresh Bearer token and reports SUCCESS on 2xx', async () => {
        const fetcher = vi.fn().mockResolvedValue(new Response('', { status: 200 }));
        const tp = tokenProviderStub();
        const exporter = new AuthInjectingLogExporter({ url: 'https://api.smoo.ai/v1/logs', tokenProvider: tp, fetcher });

        const result = await new Promise<{ code: ExportResultCode }>((resolve) => {
            exporter.export([fakeLog], resolve);
        });

        expect(result.code).toBe(ExportResultCode.SUCCESS);
        expect(fetcher).toHaveBeenCalledOnce();
        const [url, init] = fetcher.mock.calls[0]!;
        expect(url).toBe('https://api.smoo.ai/v1/logs');
        expect(init.method).toBe('POST');
        expect((init.headers as Record<string, string>).authorization).toBe('Bearer tok-123');
        // The serialized OTLP body carries the record's real W3C trace/span ids.
        expect(init.body as string).toContain('0af7651916cd43dd8448eb211c80319c');
        expect(init.body as string).toContain('b7ad6b7169203331');
    });

    it('invalidates the token and retries once on 401, then succeeds', async () => {
        const fetcher = vi
            .fn()
            .mockResolvedValueOnce(new Response('', { status: 401 }))
            .mockResolvedValueOnce(new Response('', { status: 200 }));
        const tp = tokenProviderStub();
        const exporter = new AuthInjectingLogExporter({ url: 'https://api.smoo.ai/v1/logs', tokenProvider: tp, fetcher });

        const result = await new Promise<{ code: ExportResultCode }>((resolve) => {
            exporter.export([fakeLog], resolve);
        });

        expect(result.code).toBe(ExportResultCode.SUCCESS);
        expect(tp.invalidate).toHaveBeenCalledOnce();
        expect(fetcher).toHaveBeenCalledTimes(2);
    });

    it('reports FAILED when the response is non-ok', async () => {
        const fetcher = vi.fn().mockResolvedValue(new Response('boom', { status: 503 }));
        const tp = tokenProviderStub();
        const exporter = new AuthInjectingLogExporter({ url: 'https://api.smoo.ai/v1/logs', tokenProvider: tp, fetcher });

        const result = await new Promise<{ code: ExportResultCode; error?: Error }>((resolve) => {
            exporter.export([fakeLog], resolve);
        });

        expect(result.code).toBe(ExportResultCode.FAILED);
        expect(result.error?.message).toContain('503');
    });
});
