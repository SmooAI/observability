/**
 * Node entry — Lambda / long-running Node services.
 *
 * `Client.init` here:
 *   - Spins up a node Transport (fetch + keepalive; no Beacon)
 *   - Registers `uncaughtException` / `unhandledRejection` handlers
 *   - Wires SIGTERM / SIGINT / beforeExit flushing so a Lambda container
 *     shutdown drains the in-memory queue
 *
 * The Hono middleware is exported separately so consumers wire it on their
 * app explicitly. Browser-only integrations (DOM breadcrumbs etc.) are NOT
 * imported here so the node bundle stays small.
 */
import { Client } from '../client';
import { registerNodeGlobalHandlers } from './global-handlers';
import { makeNodeTransport } from './transport';

export { Client, Scope, withScope, getCurrentScope } from '../index';
export * from '../types';
export { parseStack } from '../stack-parser';
export { registerNodeGlobalHandlers, _resetNodeGlobalHandlersForTests } from './global-handlers';
export { makeNodeTransport } from './transport';
export { observabilityMiddleware } from './middleware';
export type { ObservabilityMiddlewareOptions } from './middleware';

// Auto-wire on init — mirrors the browser entry's behavior so consumers only
// need to call `Client.init({ dsn, environment, release })`. Set
// `autoInstrumentation: false` to opt out of process error handlers (e.g.
// when the host app wants to install its own).
const originalInit = Client.init.bind(Client);
Client.init = (options) => {
    originalInit(options);
    const transport = makeNodeTransport(options);
    Client._registerTransport(async (batch) => {
        for (const evt of batch) transport.enqueue(evt);
    });
    if (options.autoInstrumentation !== false) {
        registerNodeGlobalHandlers({
            exitOnUncaught: false,
            flush: (timeoutMs) => {
                const flush = transport.flush();
                if (!timeoutMs) return flush;
                // Race the flush against a hard timeout so SIGTERM doesn't
                // stall the container shutdown.
                return Promise.race([
                    flush,
                    new Promise<void>((resolve) => {
                        const t = setTimeout(resolve, timeoutMs);
                        t.unref?.();
                    }),
                ]);
            },
        });
    }
};
