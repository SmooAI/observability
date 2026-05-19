/**
 * One-call bootstrap for the Smoo Observability SDK on Node.
 *
 * Customers (and Smoo's own services) wire instrumentation by importing this
 * module as a side effect at the top of their entry file:
 *
 *     import '@smooai/observability/bootstrap';
 *
 * The bootstrap reads its config from environment variables — no schema
 * imports, no SST `Resource` lookups, no Smoo-internal coupling. The
 * intent is that the *same* code path serves customer Lambdas /
 * containers / Next.js servers AND Smoo's internal compute, with the
 * only difference being where the env vars come from.
 *
 * ## Required env vars
 *
 *   SMOOAI_OBSERVABILITY_ENDPOINT   — base URL of the ingest API (e.g.
 *                                     "https://api.smoo.ai"). The SDK
 *                                     appends `/v1/traces` and
 *                                     `/v1/metrics`. May also be set per-
 *                                     signal via the standard
 *                                     `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
 *                                     / `_METRICS_ENDPOINT` env vars.
 *
 * ## Auth (pick ONE; pre-minted JWT wins if both are present)
 *
 *   SMOOAI_OBSERVABILITY_TOKEN          — pre-minted Bearer JWT. Easiest
 *                                         for local dev. Will not be
 *                                         refreshed; expires when the
 *                                         underlying JWT does.
 *
 *   --- or ---
 *
 *   SMOOAI_OBSERVABILITY_AUTH_URL       — base URL of the OAuth /token
 *                                         endpoint (e.g.
 *                                         "https://auth.smoo.ai"). SDK
 *                                         posts to `${URL}/token`.
 *   SMOOAI_OBSERVABILITY_CLIENT_ID      — M2M client id.
 *   SMOOAI_OBSERVABILITY_CLIENT_SECRET  — M2M client secret (the `sk_*`
 *                                         minted by Smoo's M2M flow).
 *
 *   When all three are set the SDK runs the standard `client_credentials`
 *   grant against `/token`, caches the resulting JWT, and re-mints in
 *   the background every ~55 minutes (under the 1h openauth TTL). The
 *   OTLP exporters read the header value by reference, so a refreshed
 *   token starts being used on the next export with no exporter restart.
 *
 * ## Optional env vars
 *
 *   SMOOAI_OBSERVABILITY_SERVICE_NAME   — defaults to "smoo-service".
 *                                         Surfaced as OTel `service.name`.
 *   SMOOAI_OBSERVABILITY_ENVIRONMENT    — defaults to `STAGE` /
 *                                         `NODE_ENV` / "unknown".
 *   SMOOAI_OBSERVABILITY_RELEASE        — defaults to `GIT_SHA` /
 *                                         `LAMBDA_FUNCTION_VERSION` /
 *                                         "dev".
 *   SMOOAI_OBSERVABILITY_DISABLED       — set to "1"/"true" to skip
 *                                         bootstrap entirely (useful in
 *                                         tests).
 *
 * ## Behavior
 *
 *   - Idempotent: calling `bootstrapObservability()` twice returns the
 *     same handle. Side-effect import (`import '@smooai/observability/
 *     bootstrap'`) runs the bootstrap exactly once per process.
 *   - Never throws: missing config, mint failures, and OTel init errors
 *     are logged to stderr and the SDK falls back to a no-op exporter.
 *     The host application keeps running.
 *
 * SMOODEV-1067.
 */

import { Client } from '../node';
import { setupOtelSdk, type OtelSdkHandle, type SetupOtelOptions } from '../otel';

const TOKEN_REFRESH_INTERVAL_MS = 55 * 60 * 1000; // < openauth's 1h JWT TTL

export interface BootstrapResult {
    /** Whether the bootstrap actually ran (false = disabled or already-installed). */
    installed: boolean;
    /** OTel SDK handle — flush / shutdown hooks. `null` if init failed or was skipped. */
    otel: OtelSdkHandle | null;
    /** Stops the background token-refresh timer. No-op if no timer was armed. */
    stopRefresh: () => void;
}

let bootstrapped: BootstrapResult | null = null;

/**
 * Run the bootstrap explicitly. Most callers should use the side-effect
 * import (`import '@smooai/observability/bootstrap'`) instead — but
 * tests and advanced callers can use this to override env defaults.
 */
export function bootstrapObservability(overrides: Partial<BootstrapEnv> = {}): BootstrapResult {
    if (bootstrapped) return bootstrapped;

    const env: BootstrapEnv = {
        endpoint: overrides.endpoint ?? process.env.SMOOAI_OBSERVABILITY_ENDPOINT,
        tracesEndpoint: overrides.tracesEndpoint ?? process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
        metricsEndpoint: overrides.metricsEndpoint ?? process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
        token: overrides.token ?? process.env.SMOOAI_OBSERVABILITY_TOKEN,
        authUrl: overrides.authUrl ?? process.env.SMOOAI_OBSERVABILITY_AUTH_URL,
        clientId: overrides.clientId ?? process.env.SMOOAI_OBSERVABILITY_CLIENT_ID,
        clientSecret: overrides.clientSecret ?? process.env.SMOOAI_OBSERVABILITY_CLIENT_SECRET,
        serviceName: overrides.serviceName ?? process.env.SMOOAI_OBSERVABILITY_SERVICE_NAME ?? 'smoo-service',
        environment: overrides.environment ?? process.env.SMOOAI_OBSERVABILITY_ENVIRONMENT ?? process.env.STAGE ?? process.env.NODE_ENV,
        release:
            overrides.release ??
            process.env.SMOOAI_OBSERVABILITY_RELEASE ??
            process.env.GIT_SHA ??
            process.env.LAMBDA_FUNCTION_VERSION ??
            'dev',
        disabled: overrides.disabled ?? truthy(process.env.SMOOAI_OBSERVABILITY_DISABLED),
    };

    if (env.disabled) {
        bootstrapped = { installed: false, otel: null, stopRefresh: () => {} };
        return bootstrapped;
    }

    // The headers object is held by reference inside the OTLP exporter —
    // mutating it updates every subsequent export without rebuilding the
    // exporter. That's how we refresh JWTs in-place without disturbing
    // the SDK pipeline.
    const sharedHeaders: Record<string, string> = {};

    let stopRefresh = () => {};
    try {
        if (env.token) {
            sharedHeaders.authorization = `Bearer ${env.token}`;
        } else if (env.authUrl && env.clientId && env.clientSecret) {
            stopRefresh = startTokenRefresh({
                authUrl: env.authUrl,
                clientId: env.clientId,
                clientSecret: env.clientSecret,
                onToken: (token) => {
                    sharedHeaders.authorization = `Bearer ${token}`;
                },
            });
        } else {
            // Neither auth mode configured. SDK still starts; exports will
            // 401 against gated ingest URLs. Better than crashing the host.
            warn('bootstrap: no auth configured (set SMOOAI_OBSERVABILITY_TOKEN or _AUTH_URL/_CLIENT_ID/_CLIENT_SECRET); OTLP exports will be unauthenticated');
        }

        const tracesEndpoint = env.tracesEndpoint ?? (env.endpoint ? `${stripTrailingSlash(env.endpoint)}/v1/traces` : undefined);
        const metricsEndpoint = env.metricsEndpoint ?? (env.endpoint ? `${stripTrailingSlash(env.endpoint)}/v1/metrics` : undefined);

        // Set process.env so any *other* OTel-aware code in the process
        // (e.g. third-party libraries that read the env directly) sees the
        // same endpoints. setupOtelSdk reads env too, so this also covers
        // the case where someone passes neither option nor env explicitly.
        if (tracesEndpoint && !process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT) {
            process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = tracesEndpoint;
        }
        if (metricsEndpoint && !process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT) {
            process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT = metricsEndpoint;
        }

        const otelOptions: SetupOtelOptions = {
            serviceName: env.serviceName ?? 'smoo-service',
            environment: env.environment,
            release: env.release,
            otlpEndpoint: tracesEndpoint,
            otlpHeaders: sharedHeaders,
        };

        const otel = setupOtelSdk(otelOptions);
        Client.init({
            dsn: process.env.OBSERVABILITY_DSN ?? '',
            environment: env.environment ?? 'unknown',
            release: env.release,
        });

        bootstrapped = { installed: true, otel, stopRefresh };
    } catch (err) {
        warn(`bootstrap: SDK init failed: ${err instanceof Error ? err.message : String(err)}`);
        stopRefresh();
        bootstrapped = { installed: false, otel: null, stopRefresh: () => {} };
    }

    return bootstrapped;
}

/** Reset state for tests. NOT exported from the package entry. */
export function _resetBootstrapForTests(): void {
    if (bootstrapped) bootstrapped.stopRefresh();
    bootstrapped = null;
}

export interface BootstrapEnv {
    endpoint?: string;
    tracesEndpoint?: string;
    metricsEndpoint?: string;
    token?: string;
    authUrl?: string;
    clientId?: string;
    clientSecret?: string;
    serviceName?: string;
    environment?: string;
    release?: string;
    disabled?: boolean;
}

interface RefreshConfig {
    authUrl: string;
    clientId: string;
    clientSecret: string;
    onToken: (token: string) => void;
    /** Test seam — override the timer to run synchronously. */
    schedule?: (cb: () => void, ms: number) => { unref?: () => void };
    /** Test seam — override the HTTP call. */
    fetcher?: typeof fetch;
}

function startTokenRefresh(config: RefreshConfig): () => void {
    const scheduler = config.schedule ?? ((cb, ms) => setInterval(cb, ms));
    const f = config.fetcher ?? fetch;

    let stopped = false;
    let timer: { unref?: () => void } | undefined;

    const refresh = async () => {
        if (stopped) return;
        try {
            const res = await f(`${stripTrailingSlash(config.authUrl)}/token`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
                body: new URLSearchParams({
                    grant_type: 'client_credentials',
                    provider: 'client_credentials',
                    client_id: config.clientId,
                    client_secret: config.clientSecret,
                }).toString(),
            });
            if (!res.ok) {
                warn(`bootstrap: token mint failed (${res.status}); will retry on next refresh tick`);
                return;
            }
            const body = (await res.json()) as { access_token?: string };
            if (body.access_token) {
                config.onToken(body.access_token);
            } else {
                warn('bootstrap: token endpoint returned no access_token');
            }
        } catch (err) {
            warn(`bootstrap: token mint error: ${err instanceof Error ? err.message : String(err)}`);
        }
    };

    // Fire-and-forget the initial mint so the bootstrap return is sync.
    // Exports that happen in the first ~100ms will be unauthenticated;
    // that's a negligible window vs. the lifetime of any meaningful
    // process and well within OTel's retry budget.
    void refresh();

    timer = scheduler(() => {
        void refresh();
    }, TOKEN_REFRESH_INTERVAL_MS);
    timer.unref?.();

    return () => {
        stopped = true;
        if (timer && typeof (timer as unknown as { close?: () => void }).close === 'function') {
            (timer as unknown as { close: () => void }).close();
        } else if (timer && typeof (timer as unknown as { unref?: () => void }).unref === 'function') {
            // setInterval handle — use clearInterval against it.
            clearInterval(timer as unknown as ReturnType<typeof setInterval>);
        }
    };
}

function stripTrailingSlash(url: string): string {
    return url.endsWith('/') ? url.slice(0, -1) : url;
}

function truthy(s: string | undefined): boolean {
    if (!s) return false;
    return s === '1' || s.toLowerCase() === 'true';
}

function warn(message: string): void {
    // Use stderr directly — no @smooai/logger dep, no console.warn (some
    // edge runtimes strip it). Single line, prefixed for grep-ability.
    try {
        process.stderr.write(`[@smooai/observability/bootstrap] ${message}\n`);
    } catch {
        /* don't crash if even stderr is unavailable */
    }
}

// Side-effect entry: `import '@smooai/observability/bootstrap'` runs the
// bootstrap exactly once per process. The function is also exported (above)
// so tests + advanced callers can pass overrides — the idempotent guard
// inside `bootstrapObservability` makes the double-call safe.
bootstrapObservability();
