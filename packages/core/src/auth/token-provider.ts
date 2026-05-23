/**
 * OAuth2 client_credentials token provider — direct port of
 * @smooai/config's `TokenProvider` so the observability SDK
 * authenticates against api.smoo.ai exactly the same way every other
 * smooai client does.
 *
 * Why we need this: the previous bootstrap minted a token ONCE at SDK
 * init and stuck it in a shared `sharedHeaders.authorization` map. The
 * OTel JS HTTP exporter (v0.55) `mergeHeaders`-es that map at
 * construction (Object.assign), so the original snapshot lives forever
 * — when the token expires after an hour, every subsequent /v1/traces
 * + /v1/metrics export 401s, silently dropping spans. Voice ECS
 * containers ran for hours past expiry and lost everything; warm
 * Lambdas inherited stale snapshots between cold starts.
 *
 * The TokenProvider here is consulted at *request* time by the custom
 * OTLP exporter (`auth-injecting-exporter.ts`) — no snapshot, no
 * staleness. Cached in memory until 60s before expiry, then refreshed.
 * Concurrent callers during a refresh share one in-flight request.
 *
 * Server contract:
 *
 *     POST {authUrl}/token
 *     Content-Type: application/x-www-form-urlencoded
 *
 *     grant_type=client_credentials
 *     provider=client_credentials
 *     client_id=<uuid>
 *     client_secret=sk_...
 *
 * SMOODEV-1206 — closes the SMOODEV-1128 follow-up gap noted in
 * bootstrap/index.ts.
 */

export interface TokenProviderOptions {
    /** OAuth issuer base URL (no trailing slash required). E.g. `https://auth.smoo.ai`. */
    authUrl: string;
    /** OAuth client ID. */
    clientId: string;
    /** OAuth client secret. */
    clientSecret: string;
    /**
     * Seconds before expiry to proactively refresh. Defaults to 60s — matches
     * @smooai/config TokenProvider so behavior is identical across SDKs.
     */
    refreshWindowSec?: number;
    /** Test seam — override the fetch impl. */
    fetcher?: typeof fetch;
}

interface CachedToken {
    accessToken: string;
    /** Unix epoch seconds when the token expires. */
    expiresAt: number;
}

export class TokenProvider {
    private readonly authUrl: string;
    private readonly clientId: string;
    private readonly clientSecret: string;
    private readonly refreshWindowSec: number;
    private readonly fetcher: typeof fetch;
    private cached?: CachedToken;
    private inflight?: Promise<string>;
    private nowMs: () => number = () => Date.now();

    constructor(options: TokenProviderOptions) {
        if (!options.authUrl) throw new Error('@smooai/observability: TokenProvider requires authUrl');
        if (!options.clientId) throw new Error('@smooai/observability: TokenProvider requires clientId');
        if (!options.clientSecret) throw new Error('@smooai/observability: TokenProvider requires clientSecret');
        this.authUrl = options.authUrl.replace(/\/+$/, '');
        this.clientId = options.clientId;
        this.clientSecret = options.clientSecret;
        this.refreshWindowSec = options.refreshWindowSec ?? 60;
        this.fetcher = options.fetcher ?? fetch;
    }

    /**
     * Returns a valid OAuth access token, refreshing if the cached value is
     * missing, expired, or within `refreshWindowSec` of expiry.
     *
     * Concurrent callers during a refresh share one in-flight request to
     * avoid duplicate token exchanges (and the rate-limit churn that would
     * cause on a warm Lambda servicing N parallel span exports).
     */
    async getAccessToken(): Promise<string> {
        if (!this.shouldRefresh()) return this.cached!.accessToken;
        if (this.inflight) return this.inflight;
        this.inflight = this.refresh().finally(() => {
            this.inflight = undefined;
        });
        return this.inflight;
    }

    /**
     * Drop the cached token. Call this when an export observes a 401 so the
     * next attempt re-mints. Used by the custom exporter's retry path.
     */
    invalidate(): void {
        this.cached = undefined;
    }

    private shouldRefresh(): boolean {
        if (!this.cached) return true;
        const nowSec = Math.floor(this.nowMs() / 1000);
        return nowSec >= this.cached.expiresAt - this.refreshWindowSec;
    }

    private async refresh(): Promise<string> {
        const res = await this.fetcher(`${this.authUrl}/token`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
            body: new URLSearchParams({
                grant_type: 'client_credentials',
                provider: 'client_credentials',
                client_id: this.clientId,
                client_secret: this.clientSecret,
            }).toString(),
        });
        if (!res.ok) {
            const body = await res.text().catch(() => '<unreadable>');
            throw new Error(`@smooai/observability: OAuth token exchange failed: HTTP ${res.status} ${body}`);
        }
        const body = (await res.json()) as { access_token?: string; expires_in?: number };
        if (!body.access_token) {
            throw new Error('@smooai/observability: OAuth token endpoint returned no access_token');
        }
        const expiresIn = typeof body.expires_in === 'number' ? body.expires_in : 3600;
        const nowSec = Math.floor(this.nowMs() / 1000);
        this.cached = {
            accessToken: body.access_token,
            expiresAt: nowSec + expiresIn,
        };
        return body.access_token;
    }

    /** @internal test seam — overrides the time source. */
    _setNowForTests(now: () => number): void {
        this.nowMs = now;
    }
}
