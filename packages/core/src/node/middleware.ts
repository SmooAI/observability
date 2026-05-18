/**
 * Hono-shaped observability middleware.
 *
 * For each request:
 *   1. Read user/org/session from `c.get('auth')` if the host app populated it
 *      (or via a caller-supplied `contextProvider`) and attach to the scope.
 *   2. Add a "request" context (method, path, headers subset, request id).
 *   3. Wrap the handler chain in `withScope(...)` so any `captureException`
 *      fired from a downstream handler picks up that request's identity.
 *   4. On thrown errors, call `captureException` BEFORE re-throwing so Hono's
 *      onError still gets to render the response.
 *
 * Why no AsyncLocalStorage in v1: under Lambda, each invocation is single-
 * request, so the module-scope singleton stack from `core/scope.ts` is safe.
 * If we ever target a long-lived multi-tenant Node server we'll need to swap
 * scope.ts over to ALS — tracked as a follow-up under the SDK epic.
 *
 * Naming follows Hono's middleware conventions but the type is structurally
 * compatible (we don't import `hono` from a core package).
 */

import { Client } from '../client';
import { withScope } from '../scope';

interface HonoCtxLike {
    req: {
        method: string;
        path?: string;
        url?: string;
        header: (name: string) => string | undefined;
    };
    get: (key: string) => unknown;
    set?: (key: string, value: unknown) => void;
    res?: { status?: number };
}

type HonoNext = () => Promise<void>;

export interface ObservabilityMiddlewareOptions {
    /**
     * Pull user identity out of the Hono context. Default reads `c.get('auth')`
     * shape (matches `@smooai/auth`'s authMiddleware output). Return `null` to
     * skip — the middleware will still capture errors but with no user.
     */
    resolveUser?: (c: HonoCtxLike) => { id?: string; orgId?: string; sessionId?: string } | null;
    /**
     * Header names whose values are recorded on the request context. Defaults
     * to a conservative allowlist that's safe to send to the ingest backend.
     */
    requestHeaderAllowlist?: string[];
}

const DEFAULT_HEADER_ALLOWLIST = ['user-agent', 'referer', 'x-request-id', 'x-trace-id', 'x-correlation-id'];

function defaultResolveUser(c: HonoCtxLike): { id?: string; orgId?: string; sessionId?: string } | null {
    const auth = c.get('auth') as { userId?: string; orgId?: string; sessionId?: string } | undefined;
    if (!auth) return null;
    return { id: auth.userId, orgId: auth.orgId, sessionId: auth.sessionId };
}

export function observabilityMiddleware(opts: ObservabilityMiddlewareOptions = {}) {
    const resolveUser = opts.resolveUser ?? defaultResolveUser;
    const allowlist = (opts.requestHeaderAllowlist ?? DEFAULT_HEADER_ALLOWLIST).map((h) => h.toLowerCase());

    return async function smooObservabilityMiddleware(c: HonoCtxLike, next: HonoNext): Promise<void> {
        if (!Client._isInitialized()) {
            // SDK not initialized — pass through so the host app isn't blocked.
            await next();
            return;
        }

        await withScope(async (scope) => {
            try {
                const user = resolveUser(c);
                if (user) scope.setUser(user);

                const headers: Record<string, string> = {};
                for (const name of allowlist) {
                    const v = c.req.header(name);
                    if (v) headers[name] = v;
                }
                scope.setContext('request', {
                    method: c.req.method,
                    path: c.req.path ?? c.req.url ?? '',
                    headers,
                });
            } catch {
                /* swallow — scope hydration must not break the request */
            }

            try {
                await next();
            } catch (err) {
                try {
                    Client.captureException(err, { tags: { source: 'hono.middleware' } });
                } catch {
                    /* swallow */
                }
                throw err;
            }
        });
    };
}
