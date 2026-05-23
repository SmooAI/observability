/**
 * Lambda / long-running Node OpenTelemetry SDK bootstrap.
 *
 * Initializes the OTel NodeSDK with OTLP/HTTP trace export and the standard
 * auto-instrumentations bundle. Designed to be a single call at the top of
 * `instrumentation.ts` (Next), the Lambda handler module, or the server entry
 * point — wherever the host runs first.
 *
 * Idempotent — calling `setupOtelSdk` twice is a no-op the second time so
 * tests and lazy boots don't accidentally double-register exporters.
 */

import { getNodeAutoInstrumentations } from '@opentelemetry/auto-instrumentations-node';
import { OTLPMetricExporter } from '@opentelemetry/exporter-metrics-otlp-http';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-http';
import { Resource } from '@opentelemetry/resources';
import { NodeSDK } from '@opentelemetry/sdk-node';
import { PeriodicExportingMetricReader } from '@opentelemetry/sdk-metrics';
import { ATTR_SERVICE_NAME, ATTR_SERVICE_VERSION } from '@opentelemetry/semantic-conventions';
import type { TokenProvider } from '../auth/token-provider';
import { AuthInjectingMetricExporter, AuthInjectingTraceExporter } from './auth-injecting-exporter';

export interface SetupOtelOptions {
    /** Service name surfaced in spans (e.g. 'smoo-backend', 'smoo-web'). */
    serviceName: string;
    /** OTLP/HTTP endpoint for traces. Default: collector / ingest URL pulled from env. */
    otlpEndpoint?: string;
    /** Auth headers for OTLP export (e.g. `x-api-key`, `authorization`). */
    otlpHeaders?: Record<string, string>;
    /** Deployment environment string ('production', 'staging', 'dev', 'local'). */
    environment?: string;
    /** Release identifier — git sha, Lambda version, package version. */
    release?: string;
    /**
     * Auto-instrumentation toggles. Pass `false` to disable a noisy module
     * (default config opts out of `fs` to keep span volume sane).
     * See @opentelemetry/auto-instrumentations-node for the full list.
     */
    instrumentationConfig?: Parameters<typeof getNodeAutoInstrumentations>[0];
    /**
     * Disable auto-instrumentations entirely. Caller can then register their
     * own selective set after `setupOtelSdk` returns.
     */
    disableAutoInstrumentations?: boolean;
    /**
     * Skip starting the SDK — useful for tests that want a constructed-but-
     * not-running instance. Default false.
     */
    skipStart?: boolean;
    /**
     * Metric export interval in milliseconds. Default 30_000 (30s). For
     * Lambda containers consider lowering to 5_000–10_000 so metrics flush
     * before the container freezes.
     */
    metricExportIntervalMs?: number;
    /**
     * SMOODEV-1206: when set, traces + metrics export via the
     * `AuthInjectingTraceExporter` / `AuthInjectingMetricExporter` which
     * pull a fresh Bearer from this provider on every request. Sidesteps
     * the OTel JS v0.55 header-snapshot bug that caused exports to 401
     * forever after the first token expired. When unset, falls back to
     * the standard OTLPTraceExporter + static `otlpHeaders` (existing
     * behavior for callers that pre-mint their own token).
     */
    tokenProvider?: TokenProvider;
    /**
     * Endpoint for metrics, if you want it different from the trace
     * endpoint base. Defaults to `${otlpEndpoint base}/v1/metrics` derived
     * from the trace URL or env vars.
     */
    otlpMetricsEndpoint?: string;
}

export interface OtelSdkHandle {
    /** The underlying NodeSDK so callers can shutdown / flush in their own lifecycle. */
    sdk: NodeSDK;
    /**
     * Force-flush spans now. Returns when the exporter has acknowledged or
     * the timeout elapses. Wired by the host into SIGTERM / `beforeExit`.
     */
    flush: (timeoutMs?: number) => Promise<void>;
    /**
     * Graceful shutdown — drains and closes the pipeline. Idempotent.
     */
    shutdown: () => Promise<void>;
}

let installed: OtelSdkHandle | null = null;

export function setupOtelSdk(options: SetupOtelOptions): OtelSdkHandle {
    if (installed) return installed;

    const traceEndpoint = options.otlpEndpoint ?? process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT ?? process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
    const metricEndpoint = options.otlpMetricsEndpoint ?? process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT ?? process.env.OTEL_EXPORTER_OTLP_ENDPOINT;

    // SMOODEV-1206: when a TokenProvider is passed, route through the
    // auth-injecting exporters. They ask the TokenProvider for a fresh
    // access_token on EVERY export call — no header snapshot, no expiry
    // drift. Otherwise fall through to the upstream OTLP exporters with
    // the caller's static otlpHeaders (legacy path).
    const traceExporter =
        options.tokenProvider && traceEndpoint
            ? new AuthInjectingTraceExporter({ url: traceEndpoint, tokenProvider: options.tokenProvider, staticHeaders: options.otlpHeaders })
            : traceEndpoint
              ? new OTLPTraceExporter({ url: traceEndpoint, headers: options.otlpHeaders })
              : new OTLPTraceExporter({ headers: options.otlpHeaders });

    // Metrics MeterProvider — same OTLP/HTTP transport, separate exporter.
    // PeriodicExportingMetricReader batches per-aggregation-period (default
    // 60s; Lambda containers may live shorter so a 30s window catches more).
    const metricExporter =
        options.tokenProvider && metricEndpoint
            ? new AuthInjectingMetricExporter({ url: metricEndpoint, tokenProvider: options.tokenProvider, staticHeaders: options.otlpHeaders })
            : metricEndpoint
              ? new OTLPMetricExporter({ url: metricEndpoint, headers: options.otlpHeaders })
              : new OTLPMetricExporter({ headers: options.otlpHeaders });
    // sdk-node and sdk-metrics ship slightly different versions of the
    // MetricReader class (private-property nominal-typing mismatch). Cast
    // through `unknown` — runtime contract is identical.
    const metricReader = new PeriodicExportingMetricReader({
        exporter: metricExporter,
        exportIntervalMillis: options.metricExportIntervalMs ?? 30_000,
    });

    const resource = new Resource({
        [ATTR_SERVICE_NAME]: options.serviceName,
        ...(options.release ? { [ATTR_SERVICE_VERSION]: options.release } : {}),
        ...(options.environment ? { 'deployment.environment.name': options.environment } : {}),
    });

    const instrumentations = options.disableAutoInstrumentations
        ? []
        : [
              getNodeAutoInstrumentations({
                  // `fs` spans drown out everything else for negligible signal.
                  '@opentelemetry/instrumentation-fs': { enabled: false },
                  ...options.instrumentationConfig,
              }),
          ];

    const sdk = new NodeSDK({
        resource,
        traceExporter,
        // Cast: sdk-node bundles an older sdk-metrics; the runtime API matches.
        metricReader: metricReader as unknown as never,
        instrumentations,
    });

    if (!options.skipStart) {
        sdk.start();
    }

    const handle: OtelSdkHandle = {
        sdk,
        async flush(timeoutMs = 2_000) {
            // NodeSDK doesn't expose a public flush; shutdown drains the exporter
            // queue. Wrap in Promise.race so a slow exporter doesn't stall SIGTERM.
            const drain = (async () => {
                try {
                    // sdk.shutdown is what we have — but it permanently closes.
                    // Prefer the exporter's `forceFlush` when available.
                    const exporterWithFlush = traceExporter as unknown as { forceFlush?: () => Promise<void> };
                    if (typeof exporterWithFlush.forceFlush === 'function') {
                        await exporterWithFlush.forceFlush();
                    }
                } catch {
                    /* swallow — flush is best-effort */
                }
            })();
            await Promise.race([
                drain,
                new Promise<void>((resolve) => {
                    const t = setTimeout(resolve, timeoutMs);
                    t.unref?.();
                }),
            ]);
        },
        async shutdown() {
            try {
                await sdk.shutdown();
            } catch {
                /* swallow */
            } finally {
                installed = null;
            }
        },
    };

    installed = handle;
    return handle;
}

/** Test seam — wipes the install guard so the next call re-initializes. */
export function _resetOtelSdkForTests(): void {
    installed = null;
}
