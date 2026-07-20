/**
 * Browser entry point. Side-effect: when consumed via the package `exports`
 * map browser condition, the universal `Client.init` resolves the browser
 * runtime (we wire that here lazily, NOT at import time, to keep tree-shaking
 * intact for users who import only types).
 */
import { Client } from '../client';
import { installFetchBreadcrumbs, installNavigationBreadcrumbs } from './breadcrumbs';
import { installConsoleErrorTap } from './console-tap';
import { registerBrowserGlobalHandlers } from './global-handlers';
import { makeBrowserTransport } from './transport';

export { Client, Scope, withScope, getCurrentScope } from '../index';
export * from '../types';
export { parseStack } from '../stack-parser';
export { registerBrowserGlobalHandlers } from './global-handlers';
export { installFetchBreadcrumbs, installNavigationBreadcrumbs } from './breadcrumbs';
export { installConsoleErrorTap } from './console-tap';
export { makeBrowserTransport } from './transport';
export { installWebVitals, recordWebVital, type InstallWebVitalsOptions } from './web-vitals';

// Auto-wire on init. The Client.init implementation lives in core/client.ts
// and stores options; we hook into it here by re-defining the init behavior
// to register integrations after the first call.
const originalInit = Client.init.bind(Client);
Client.init = (options) => {
    originalInit(options);
    if (options.autoInstrumentation !== false) {
        registerBrowserGlobalHandlers();
        installFetchBreadcrumbs();
        installNavigationBreadcrumbs();
        // Console tap is opt-in via explicit autoInstrumentation flag.
    }
    const transport = makeBrowserTransport(options);
    Client._registerTransport(async (batch) => {
        for (const evt of batch) transport.enqueue(evt);
    });
};
