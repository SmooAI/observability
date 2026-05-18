/**
 * Node-flavored Transport adapter. Mirrors `browser/transport.ts`:
 *   - No `sendBeacon` (Node has no equivalent).
 *   - No DOM lifecycle hooks — process-level SIGTERM/SIGINT flushing is wired
 *     by `registerNodeGlobalHandlers({ flush })` instead, so it's idempotent
 *     and doesn't fight the host app's signal handlers.
 *
 * Returns the underlying Transport instance so callers (e.g. node/index.ts)
 * can wire `transport.flush()` into the global-handlers lifecycle.
 */

import { Transport } from '../transport';
import type { ClientOptions } from '../types';

export function makeNodeTransport(opts: ClientOptions): Transport {
    const adapter = {
        // Node has no Beacon API; force fetch-with-keepalive path on flush.
        canBeacon: false,
        // No bindLifecycle — process signals are handled in global-handlers
        // so we don't double-register on imports of the transport alone.
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
