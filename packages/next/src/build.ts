import type { NextConfig } from 'next';

interface WithSmooObservabilityOptions {
    /** Org slug used for sourcemap upload paths. */
    org: string;
    /** Release identifier — typically GITHUB_SHA. */
    release: string;
    /** Only upload sourcemaps in CI to avoid clogging the bucket during dev. */
    uploadSourcemaps?: boolean;
    /** CI-only B2M client id, defaults to env SMOO_OBS_CI_CLIENT_ID. */
    ciClientId?: string;
    /** CI-only B2M client secret, defaults to env SMOO_OBS_CI_CLIENT_SECRET. */
    ciClientSecret?: string;
    /** Override the upload endpoint host. */
    apiHost?: string;
}

/**
 * Wrap `next.config.ts` with the Smoo Observability build-time hook. Enables
 * production sourcemaps so symbolication is possible, and (in CI) uploads
 * them to the Smoo Observability sourcemap bucket.
 *
 * ```ts
 * import { withSmooObservability } from '@smooai/observability-next/build';
 *
 * export default withSmooObservability(myNextConfig, {
 *     org: 'my-org',
 *     release: process.env.GITHUB_SHA ?? 'dev',
 *     uploadSourcemaps: process.env.CI === 'true',
 * });
 * ```
 */
export function withSmooObservability(nextConfig: NextConfig, _options: WithSmooObservabilityOptions): NextConfig {
    return {
        ...nextConfig,
        productionBrowserSourceMaps: true,
        webpack(config, ctx) {
            // TODO (SMOODEV-1067): inject a webpack plugin that runs after emit
            // and POSTs each generated .map file to the Smoo sourcemap endpoint.
            const inner = nextConfig.webpack;
            return typeof inner === 'function' ? inner(config, ctx) : config;
        },
    };
}
