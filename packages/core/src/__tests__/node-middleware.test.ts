import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Client } from '../client';
import { observabilityMiddleware } from '../node/middleware';
import { getCurrentScope } from '../scope';

function makeCtx(overrides: Partial<{ method: string; path: string; headers: Record<string, string>; auth: unknown }> = {}) {
    const headers = overrides.headers ?? {};
    return {
        req: {
            method: overrides.method ?? 'GET',
            path: overrides.path ?? '/x',
            header: (name: string) => headers[name.toLowerCase()],
        },
        get: (key: string) => (key === 'auth' ? overrides.auth : undefined),
    };
}

describe('observabilityMiddleware', () => {
    beforeEach(() => {
        Client.init({ dsn: 'https://ingest.example/wh/o/t' });
    });

    it('passes through when SDK is not initialized', async () => {
        // Reset SDK init state by re-importing — easiest: simulate with a private flag flip.
        // Since the existing Client doesn't expose a reset hook, use a fresh middleware path:
        const mw = observabilityMiddleware();
        const captureSpy = vi.spyOn(Client, '_isInitialized').mockReturnValue(false);
        const next = vi.fn().mockResolvedValue(undefined);
        await mw(makeCtx(), next);
        expect(next).toHaveBeenCalled();
        captureSpy.mockRestore();
    });

    it('hydrates user from c.get("auth") onto the scope', async () => {
        const mw = observabilityMiddleware();
        let scopeUser: unknown;
        const next = vi.fn().mockImplementation(async () => {
            // Capture the scope state inside the request — withScope's cloned
            // scope is active here.
            scopeUser = (getCurrentScope() as unknown as { user?: unknown }).user;
        });
        await mw(makeCtx({ auth: { userId: 'u1', orgId: 'org1', sessionId: 's1' } }), next);
        expect(scopeUser).toEqual({ id: 'u1', orgId: 'org1', sessionId: 's1' });
    });

    it('captures thrown errors and re-throws so onError still fires', async () => {
        const captureSpy = vi.spyOn(Client, 'captureException').mockReturnValue('evt-id');
        const mw = observabilityMiddleware();
        const err = new Error('handler boom');
        const next = vi.fn().mockRejectedValue(err);
        await expect(mw(makeCtx(), next)).rejects.toThrow('handler boom');
        expect(captureSpy).toHaveBeenCalledWith(err, { tags: { source: 'hono.middleware' } });
        captureSpy.mockRestore();
    });

    it('records allow-listed request headers on the request context', async () => {
        const mw = observabilityMiddleware();
        let requestCtx: unknown;
        const next = vi.fn().mockImplementation(async () => {
            requestCtx = (getCurrentScope() as unknown as { contexts?: Record<string, unknown> }).contexts?.request;
        });
        await mw(
            makeCtx({
                method: 'POST',
                path: '/api/x',
                headers: { 'x-request-id': 'rid-1', 'x-correlation-id': 'cid-1', 'should-not-leak': 'value' },
            }),
            next,
        );
        expect(requestCtx).toMatchObject({
            method: 'POST',
            path: '/api/x',
            headers: { 'x-request-id': 'rid-1', 'x-correlation-id': 'cid-1' },
        });
        // confirm non-allowlisted header didn't leak
        expect(((requestCtx as { headers: Record<string, string> }).headers as Record<string, string>)['should-not-leak']).toBeUndefined();
    });

    it('pops the scope after async work completes (no leak)', async () => {
        const mw = observabilityMiddleware();
        const before = (getCurrentScope() as unknown as { user?: unknown }).user;
        await mw(makeCtx({ auth: { userId: 'temp' } }), async () => {
            await new Promise((r) => setTimeout(r, 5));
        });
        const after = (getCurrentScope() as unknown as { user?: unknown }).user;
        expect(after).toEqual(before);
    });
});
