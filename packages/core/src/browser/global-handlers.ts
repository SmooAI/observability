import { Client } from '../client';

/**
 * Register browser-side global error capture: `window.onerror` and
 * `window.onunhandledrejection`. Composes with any existing handlers so we
 * don't clobber app code or other SDKs.
 *
 * Idempotent — calling twice is a no-op.
 */
let installed = false;

export function registerBrowserGlobalHandlers(): void {
    if (installed || typeof window === 'undefined') return;
    installed = true;

    const prevOnError = window.onerror;
    window.onerror = function smooErrorHandler(message, source, lineno, colno, error) {
        try {
            const err = error instanceof Error ? error : new Error(typeof message === 'string' ? message : 'window.onerror');
            Client.captureException(err, {
                tags: {
                    source: 'window.onerror',
                    ...(source ? { file: String(source) } : {}),
                    ...(lineno ? { lineno: String(lineno) } : {}),
                    ...(colno ? { colno: String(colno) } : {}),
                },
            });
        } catch {
            /* swallow — observability must not throw */
        }
        if (typeof prevOnError === 'function') {
            return (prevOnError as OnErrorEventHandlerNonNull).call(window, message, source, lineno, colno, error);
        }
        return false;
    };

    const prevOnUnhandled = window.onunhandledrejection;
    window.onunhandledrejection = function smooRejectionHandler(event) {
        try {
            const reason = (event as PromiseRejectionEvent).reason;
            const err = reason instanceof Error ? reason : new Error(typeof reason === 'string' ? reason : 'unhandledrejection');
            Client.captureException(err, { tags: { source: 'unhandledrejection' } });
        } catch {
            /* swallow */
        }
        if (typeof prevOnUnhandled === 'function') {
            return (prevOnUnhandled as (this: Window, ev: PromiseRejectionEvent) => unknown).call(window, event as PromiseRejectionEvent);
        }
    };
}
