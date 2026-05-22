import { Transport } from '../transport';
import type { ClientOptions } from '../types';

/**
 * Node-flavored Transport. Same batched queue as the browser variant but
 * no `sendBeacon` / lifecycle binding — Node processes typically have a
 * clean shutdown path via flush hooks instead of page-unload events. The
 * underlying fetch is global in Node 18+, which the universal Transport
 * uses internally.
 *
 * SMOODEV-1148: this gets registered in Node init so `Client.captureException`
 * fans out to BOTH the OTel-native captureHandler (span events) AND the
 * webhook POST (errorEvents table → Errors dashboard).
 */
export function makeNodeTransport(opts: ClientOptions): Transport {
    const adapter = {
        canBeacon: false,
    };
    return new Transport(
        {
            dsn: opts.dsn,
            flushIntervalMs: opts.flushIntervalMs,
            maxBatchSize: opts.maxBatchSize,
            maxQueueSize: opts.maxQueueSize,
        },
        adapter,
    );
}
