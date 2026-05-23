/**
 * Custom OTLP/HTTP/JSON span + metric exporters that authenticate against
 * api.smoo.ai via the @smooai/observability TokenProvider — fresh token on
 * EVERY export, no header snapshot, no expiry drift.
 *
 * Why we need our own exporters:
 *
 * The upstream @opentelemetry/exporter-trace-otlp-http (v0.55) takes a
 * `headers: Record<string, string>` option and `Object.assign`s it into
 * an internal state object at construction time. Once you've created the
 * exporter, mutating the original headers map is invisible to the
 * exporter — it's already snapshotted. So the previous bootstrap pattern
 * of "mint a token, stick it in a shared headers map, schedule a
 * background refresh that updates the same map" silently broke after the
 * first token expired (~1h). Every subsequent OTLP export 401'd and the
 * spans were lost.
 *
 * Fix: hand-roll the SpanExporter / PushMetricExporter contracts. Each
 * export() call:
 *   1. Asks the TokenProvider for a token (cache-hit if not expiring,
 *      single in-flight mint if expired).
 *   2. POSTs the OTLP JSON body with a fresh Authorization header.
 *   3. On 401, invalidates the cached token + retries ONCE.
 *
 * Wire format: OTLP/HTTP/JSON (Content-Type: application/json). The
 * @opentelemetry/otlp-transformer library handles ReadableSpan →
 * IExportTraceServiceRequest serialization and the same for metrics, so
 * we don't reinvent protobuf encoding.
 *
 * SMOODEV-1206.
 */

import { ExportResult, ExportResultCode } from '@opentelemetry/core';
import { JsonMetricsSerializer, JsonTraceSerializer } from '@opentelemetry/otlp-transformer';
import type { ResourceMetrics, PushMetricExporter } from '@opentelemetry/sdk-metrics';
import type { ReadableSpan, SpanExporter } from '@opentelemetry/sdk-trace-base';
import type { TokenProvider } from '../auth/token-provider';

interface AuthInjectingExporterOptions {
    /** Fully-qualified OTLP endpoint, e.g. `https://api.smoo.ai/v1/traces`. */
    url: string;
    /** Token provider that holds the cached access_token. Consulted per-request. */
    tokenProvider: TokenProvider;
    /** Static headers to merge onto every request (e.g. user-agent). */
    staticHeaders?: Record<string, string>;
    /** Test seam — override fetch. */
    fetcher?: typeof fetch;
    /** Per-request timeout in ms. Default 10s. */
    timeoutMs?: number;
}

abstract class BaseAuthInjectingExporter<Item> {
    protected readonly url: string;
    protected readonly tokenProvider: TokenProvider;
    protected readonly staticHeaders: Record<string, string>;
    protected readonly fetcher: typeof fetch;
    protected readonly timeoutMs: number;
    private shutdownRequested = false;

    constructor(opts: AuthInjectingExporterOptions) {
        if (!opts.url) throw new Error('@smooai/observability: AuthInjectingExporter requires url');
        this.url = opts.url;
        this.tokenProvider = opts.tokenProvider;
        this.staticHeaders = opts.staticHeaders ?? {};
        this.fetcher = opts.fetcher ?? fetch;
        this.timeoutMs = opts.timeoutMs ?? 10_000;
    }

    /**
     * Subclass plugs in the OTLP serializer for the relevant signal
     * (traces / metrics / logs).
     */
    protected abstract serialize(items: Item[]): Uint8Array;

    /**
     * The SpanExporter / PushMetricExporter `export` contract. Both
     * libraries use the same shape: (items, callback). Routes through
     * `doExport` which handles the auth + retry.
     */
    protected dispatch(items: Item[], resultCallback: (result: ExportResult) => void): void {
        if (this.shutdownRequested) {
            resultCallback({ code: ExportResultCode.FAILED, error: new Error('exporter shut down') });
            return;
        }
        if (items.length === 0) {
            resultCallback({ code: ExportResultCode.SUCCESS });
            return;
        }
        void this.doExport(items)
            .then(() => resultCallback({ code: ExportResultCode.SUCCESS }))
            .catch((error: unknown) => {
                resultCallback({ code: ExportResultCode.FAILED, error: error instanceof Error ? error : new Error(String(error)) });
            });
    }

    private async doExport(items: Item[]): Promise<void> {
        const bodyBytes = this.serialize(items);
        // JSON OTLP — decode the serializer's Uint8Array back to a string for
        // the fetch body. fetch's typed BodyInit doesn't accept Uint8Array
        // directly in lib.dom, even though the runtime does. String is fine
        // since the payload is JSON.
        const body = new TextDecoder().decode(bodyBytes);
        const attempt = async (): Promise<Response> => {
            const token = await this.tokenProvider.getAccessToken();
            const controller = new AbortController();
            const timer = setTimeout(() => controller.abort(), this.timeoutMs);
            try {
                return await this.fetcher(this.url, {
                    method: 'POST',
                    headers: {
                        ...this.staticHeaders,
                        authorization: `Bearer ${token}`,
                        'content-type': 'application/json',
                    },
                    body,
                    signal: controller.signal,
                });
            } finally {
                clearTimeout(timer);
            }
        };

        let res = await attempt();

        // 401 retry: token may have been revoked / rotated server-side. Drop
        // the cached value and re-mint once. Don't loop forever.
        if (res.status === 401) {
            this.tokenProvider.invalidate();
            res = await attempt();
        }

        if (!res.ok) {
            // Read a small slice of the body for context but don't blow up
            // on huge error pages.
            const txt = await res.text().catch(() => '<unreadable>');
            throw new Error(`OTLP export ${this.url} failed: HTTP ${res.status} ${txt.slice(0, 300)}`);
        }
    }

    async shutdown(): Promise<void> {
        this.shutdownRequested = true;
    }

    /**
     * Both SpanExporter + PushMetricExporter declare a `forceFlush()`. Our
     * exporter has no internal queue — `dispatch` posts synchronously per
     * call — so flush is a no-op.
     */
    forceFlush(): Promise<void> {
        return Promise.resolve();
    }
}

export class AuthInjectingTraceExporter extends BaseAuthInjectingExporter<ReadableSpan> implements SpanExporter {
    protected serialize(items: ReadableSpan[]): Uint8Array {
        return JsonTraceSerializer.serializeRequest(items) ?? new Uint8Array();
    }

    export(items: ReadableSpan[], resultCallback: (result: ExportResult) => void): void {
        this.dispatch(items, resultCallback);
    }
}

export class AuthInjectingMetricExporter extends BaseAuthInjectingExporter<ResourceMetrics> implements PushMetricExporter {
    protected serialize(items: ResourceMetrics[]): Uint8Array {
        // JsonMetricsSerializer takes ResourceMetrics[] directly.
        return JsonMetricsSerializer.serializeRequest(items) ?? new Uint8Array();
    }

    export(metrics: ResourceMetrics, resultCallback: (result: ExportResult) => void): void {
        this.dispatch([metrics], resultCallback);
    }

    selectAggregation = undefined;
    selectAggregationTemporality = undefined;
}
