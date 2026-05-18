/**
 * Node-side global error capture. Mirrors `browser/global-handlers.ts`:
 * intercepts `uncaughtException` and `unhandledRejection` and forwards to
 * `Client.captureException` without clobbering any listener the host app
 * already attached. Idempotent — calling twice installs once.
 *
 * Observability must never throw into user code, so every handler is wrapped
 * in try/catch and a transport / capture failure swallows silently.
 *
 * Lifecycle flushing is optional — pass `flush` so SIGTERM / SIGINT /
 * `beforeExit` drains the in-memory queue. Node index.ts wires this to the
 * Transport instance so global-handlers doesn't have to know about transports.
 */

import { Client } from '../client';

let installed = false;

export interface RegisterNodeOptions {
    /**
     * When true, after capturing an uncaughtException we re-emit `process.exit(1)`
     * on the next tick. Matches pre-Node-15 default behavior; default false
     * because Hono/Lambda handlers usually want the process to continue.
     */
    exitOnUncaught?: boolean;
    /**
     * Optional flush hook. Receives a timeout (ms) hint; should return when
     * queued events are drained or the timeout expires. Called on SIGTERM,
     * SIGINT, and `beforeExit` so a Lambda container shutdown / Node service
     * stop doesn't lose buffered events.
     */
    flush?: (timeoutMs: number) => Promise<void> | void;
}

export function registerNodeGlobalHandlers(opts: RegisterNodeOptions = {}): void {
    if (installed || typeof process === 'undefined') return;
    installed = true;

    const onUncaught = (err: unknown): void => {
        try {
            const e = err instanceof Error ? err : new Error(typeof err === 'string' ? err : 'uncaughtException');
            Client.captureException(e, { tags: { source: 'uncaughtException' } });
        } catch {
            /* swallow */
        }
        if (opts.exitOnUncaught) {
            const t = setTimeout(() => process.exit(1), 0);
            t.unref?.();
        }
    };
    process.on('uncaughtException', onUncaught);

    const onUnhandled = (reason: unknown): void => {
        try {
            const e = reason instanceof Error ? reason : new Error(typeof reason === 'string' ? reason : 'unhandledRejection');
            Client.captureException(e, { tags: { source: 'unhandledRejection' } });
        } catch {
            /* swallow */
        }
    };
    process.on('unhandledRejection', onUnhandled);

    if (opts.flush) {
        const flushFn = opts.flush;
        const onExit = (): void => {
            try {
                void Promise.resolve(flushFn(2_000)).catch(() => {});
            } catch {
                /* swallow */
            }
        };
        process.once('SIGTERM', onExit);
        process.once('SIGINT', onExit);
        process.once('beforeExit', onExit);
    }
}

/** For tests — resets the install guard so the next call re-attaches. */
export function _resetNodeGlobalHandlersForTests(): void {
    installed = false;
}
