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
 *                                     appends `/v1/traces`, `/v1/metrics`,
 *                                     and `/v1/logs`. May also be set per-
 *                                     signal via the standard
 *                                     `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
 *                                     / `_METRICS_ENDPOINT` / `_LOGS_ENDPOINT`
 *                                     env vars.
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

import { TokenProvider } from '../auth/token-provider';
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
 *
 * Returns a Promise so the initial token mint can complete before the
 * OTel SDK is constructed. The OTel HTTP exporter (v0.55+) snapshots
 * its `headers` config at construction time via `Object.assign` — so
 * the previous fire-and-forget approach left the exporter holding an
 * empty header object permanently, and every export 401'd at the
 * Bearer-auth gate. SMOODEV-1128.
 */
export async function bootstrapObservability(overrides: Partial<BootstrapEnv> = {}): Promise<BootstrapResult> {
    if (bootstrapped) return bootstrapped;

    const env: BootstrapEnv = {
        endpoint: overrides.endpoint ?? process.env.SMOOAI_OBSERVABILITY_ENDPOINT,
        tracesEndpoint: overrides.tracesEndpoint ?? process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
        metricsEndpoint: overrides.metricsEndpoint ?? process.env.OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
        logsEndpoint: overrides.logsEndpoint ?? process.env.OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
        token: overrides.token ?? process.env.SMOOAI_OBSERVABILITY_TOKEN,
        authUrl: overrides.authUrl ?? process.env.SMOOAI_OBSERVABILITY_AUTH_URL,
        clientId: overrides.clientId ?? process.env.SMOOAI_OBSERVABILITY_CLIENT_ID,
        clientSecret: overrides.clientSecret ?? process.env.SMOOAI_OBSERVABILITY_CLIENT_SECRET,
        serviceName: overrides.serviceName ?? process.env.SMOOAI_OBSERVABILITY_SERVICE_NAME ?? 'smoo-service',
        environment: overrides.environment ?? process.env.SMOOAI_OBSERVABILITY_ENVIRONMENT ?? process.env.STAGE ?? process.env.NODE_ENV,
        release: overrides.release ?? process.env.SMOOAI_OBSERVABILITY_RELEASE ?? process.env.GIT_SHA ?? process.env.LAMBDA_FUNCTION_VERSION ?? 'dev',
        disabled: overrides.disabled ?? truthy(process.env.SMOOAI_OBSERVABILITY_DISABLED),
    };

    if (env.disabled) {
        bootstrapped = { installed: false, otel: null, stopRefresh: () => {} };
        return bootstrapped;
    }

    // SMOODEV-1206: token auth flow. Previously this minted once + scheduled
    // a refresh, but the OTel JS v0.55 OTLP HTTP exporter Object.assigns its
    // headers at construction time — the original snapshot lived forever and
    // every export 401'd after the first token expired (~1h). Fix: pass a
    // TokenProvider into setupOtelSdk; the custom AuthInjectingTraceExporter
    // pulls a fresh Bearer from it on every request. Mirrors how every other
    // smoo SDK (config, fetch, file) handles client_credentials auth.
    //
    // The static-token + sharedHeaders path is retained for the
    // SMOOAI_OBSERVABILITY_TOKEN escape hatch — pre-minted tokens stay
    // snapshotted (caller's responsibility to keep them fresh).
    const sharedHeaders: Record<string, string> = {};
    let tokenProvider: TokenProvider | undefined;

    let stopRefresh = () => {};
    try {
        if (env.token) {
            sharedHeaders.authorization = `Bearer ${env.token}`;
        } else if (env.authUrl && env.clientId && env.clientSecret) {
            // SMOODEV-1206: construct the per-request TokenProvider and
            // also do a synchronous warm-up mint so the very first OTLP
            // export doesn't pay the round-trip latency.
            tokenProvider = new TokenProvider({
                authUrl: env.authUrl,
                clientId: env.clientId,
                clientSecret: env.clientSecret,
            });
            try {
                await tokenProvider.getAccessToken();
            } catch (mintErr) {
                warn(
                    `bootstrap: initial token mint failed; OTLP exports will retry on first export: ${mintErr instanceof Error ? mintErr.message : String(mintErr)}`,
                );
            }
        } else {
            // Neither auth mode configured. SDK still starts; exports will
            // 401 against gated ingest URLs. Better than crashing the host.
            warn('bootstrap: no auth configured (set SMOOAI_OBSERVABILITY_TOKEN or _AUTH_URL/_CLIENT_ID/_CLIENT_SECRET); OTLP exports will be unauthenticated');
        }

        const tracesEndpoint = env.tracesEndpoint ?? (env.endpoint ? `${stripTrailingSlash(env.endpoint)}/v1/traces` : undefined);
        const metricsEndpoint = env.metricsEndpoint ?? (env.endpoint ? `${stripTrailingSlash(env.endpoint)}/v1/metrics` : undefined);
        const logsEndpoint = env.logsEndpoint ?? (env.endpoint ? `${stripTrailingSlash(env.endpoint)}/v1/logs` : undefined);

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
        if (logsEndpoint && !process.env.OTEL_EXPORTER_OTLP_LOGS_ENDPOINT) {
            process.env.OTEL_EXPORTER_OTLP_LOGS_ENDPOINT = logsEndpoint;
        }

        const otelOptions: SetupOtelOptions = {
            serviceName: env.serviceName ?? 'smoo-service',
            environment: env.environment,
            release: env.release,
            otlpEndpoint: tracesEndpoint,
            otlpMetricsEndpoint: metricsEndpoint,
            otlpLogsEndpoint: logsEndpoint,
            otlpHeaders: sharedHeaders,
            tokenProvider,
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
    logsEndpoint?: string;
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

/**
 * Perform a single `client_credentials` token mint. Returns the
 * `access_token` on success, or `undefined` on any failure (and warns
 * to stderr). Used both for the initial sync mint at bootstrap time
 * and for the background refresh ticks.
 */
async function mintToken(opts: { authUrl: string; clientId: string; clientSecret: string; fetcher?: typeof fetch }): Promise<string | undefined> {
    const f = opts.fetcher ?? fetch;
    try {
        const res = await f(`${stripTrailingSlash(opts.authUrl)}/token`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
            body: new URLSearchParams({
                grant_type: 'client_credentials',
                provider: 'client_credentials',
                client_id: opts.clientId,
                client_secret: opts.clientSecret,
            }).toString(),
        });
        if (!res.ok) {
            warn(`bootstrap: token mint failed (${res.status})`);
            return undefined;
        }
        const body = (await res.json()) as { access_token?: string };
        if (!body.access_token) {
            warn('bootstrap: token endpoint returned no access_token');
            return undefined;
        }
        return body.access_token;
    } catch (err) {
        warn(`bootstrap: token mint error: ${err instanceof Error ? err.message : String(err)}`);
        return undefined;
    }
}

/**
 * Arm a background timer that re-mints the token every
 * TOKEN_REFRESH_INTERVAL_MS. Returns a stop function.
 *
 * Note: subsequent mints update `sharedHeaders` via `onToken`, but
 * because OTel v0.55+ snapshots headers at construction, the live
 * exporter will not pick up the refreshed value (SMOODEV-1128). In
 * practice long-lived containers should be redeployed within the
 * underlying JWT TTL (1h); this timer remains for future use once
 * a header-getter transport is in place.
 */
function scheduleTokenRefresh(config: RefreshConfig): () => void {
    const scheduler = config.schedule ?? ((cb, ms) => setInterval(cb, ms));
    let stopped = false;
    let timer: { unref?: () => void } | undefined;

    timer = scheduler(() => {
        if (stopped) return;
        void mintToken({
            authUrl: config.authUrl,
            clientId: config.clientId,
            clientSecret: config.clientSecret,
            fetcher: config.fetcher,
        }).then((token) => {
            if (token && !stopped) config.onToken(token);
        });
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
//
// SMOODEV-1128: top-level await is required because the initial token
// mint must complete before the OTel exporter is constructed (the
// exporter snapshots headers at construction; see the body comment for
// the OTel v0.55 Object.assign behavior). target es2022 in tsdown
// supports top-level await in ESM.
await bootstrapObservability();
