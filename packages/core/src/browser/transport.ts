import { Transport } from '../transport';
import type { ClientOptions } from '../types';

/**
 * Build a browser-flavored Transport: keepalive fetch, `sendBeacon` on
 * `pagehide`, bound to `visibilitychange` for the modern browser-lifecycle path.
 */
export function makeBrowserTransport(opts: ClientOptions): Transport {
    const adapter = {
        canBeacon: typeof navigator !== 'undefined' && typeof navigator.sendBeacon === 'function',
        beacon: typeof navigator !== 'undefined' && typeof navigator.sendBeacon === 'function' ? navigator.sendBeacon.bind(navigator) : undefined,
        bindLifecycle: (onPageHide: () => void) => {
            if (typeof window === 'undefined') return;
            // `pagehide` is the modern unload event; visibilitychange is the bfcache-friendly path.
            window.addEventListener('pagehide', onPageHide, { capture: true });
            window.addEventListener('visibilitychange', () => {
                if (document.visibilityState === 'hidden') onPageHide();
            });
        },
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
