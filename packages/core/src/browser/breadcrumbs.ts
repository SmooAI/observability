import { Client } from '../client';
import { scrubString } from '../pii';

/**
 * Install passive breadcrumb wrappers for fetch + navigation. They never throw
 * into user code; failures are swallowed.
 */
let fetchInstalled = false;
let navInstalled = false;

export function installFetchBreadcrumbs(): void {
    if (fetchInstalled || typeof window === 'undefined' || typeof window.fetch !== 'function') return;
    fetchInstalled = true;
    const originalFetch = window.fetch.bind(window);
    window.fetch = async function smooFetch(input: RequestInfo | URL, init?: RequestInit) {
        const started = Date.now();
        const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : (input as Request).url;
        const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
        try {
            const res = await originalFetch(input as RequestInfo, init);
            Client.addBreadcrumb(
                'fetch',
                `${method} ${scrubString(url)} ${res.status}`,
                {
                    method,
                    url: scrubString(url),
                    status: res.status,
                    duration_ms: Date.now() - started,
                },
                res.ok ? 'info' : 'warning',
            );
            return res;
        } catch (err) {
            Client.addBreadcrumb(
                'fetch',
                `${method} ${scrubString(url)} threw`,
                {
                    method,
                    url: scrubString(url),
                    error: err instanceof Error ? err.message : String(err),
                    duration_ms: Date.now() - started,
                },
                'error',
            );
            throw err;
        }
    } as typeof window.fetch;
}

export function installNavigationBreadcrumbs(): void {
    if (navInstalled || typeof window === 'undefined') return;
    navInstalled = true;

    const pushBreadcrumb = (kind: string, to: string) => {
        Client.addBreadcrumb('navigation', `${kind} → ${to}`, { kind, to });
    };

    // history.pushState / replaceState
    const wrap = <K extends 'pushState' | 'replaceState'>(name: K) => {
        const original = history[name];
        history[name] = function smooHistory(this: History, ...args: Parameters<History[K]>) {
            const result = (original as (...a: unknown[]) => unknown).apply(this, args);
            const url = String(args[2] ?? window.location.href);
            pushBreadcrumb(name, url);
            return result;
        } as History[K];
    };
    wrap('pushState');
    wrap('replaceState');

    window.addEventListener('popstate', () => pushBreadcrumb('popstate', window.location.href));
    window.addEventListener('hashchange', () => pushBreadcrumb('hashchange', window.location.href));
}
