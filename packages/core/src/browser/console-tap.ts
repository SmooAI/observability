import { Client } from '../client';

let installed = false;

/**
 * Optional tap on `console.error`. Captures the first argument (string or
 * Error) as a level=error event. Disabled by default — turn on with
 * `Client.init({ autoInstrumentation: true })`.
 */
export function installConsoleErrorTap(): void {
    if (installed || typeof console === 'undefined') return;
    installed = true;
    const original = console.error.bind(console);
    console.error = (...args: unknown[]) => {
        try {
            const first = args[0];
            if (first instanceof Error) {
                Client.captureException(first, { tags: { source: 'console.error' } });
            } else if (typeof first === 'string') {
                Client.captureMessage(first, 'error');
            }
        } catch {
            /* swallow */
        }
        return original(...args);
    };
}
