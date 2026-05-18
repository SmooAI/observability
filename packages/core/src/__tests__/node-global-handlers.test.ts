import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Client } from '../client';
import { _resetNodeGlobalHandlersForTests, registerNodeGlobalHandlers } from '../node/global-handlers';

describe('registerNodeGlobalHandlers', () => {
    let captureSpy: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        _resetNodeGlobalHandlersForTests();
        captureSpy = vi.fn(() => 'evt-id' as string | undefined);
        vi.spyOn(Client, 'captureException').mockImplementation(captureSpy as never);
    });

    afterEach(() => {
        vi.restoreAllMocks();
        process.removeAllListeners('uncaughtException');
        process.removeAllListeners('unhandledRejection');
        process.removeAllListeners('SIGTERM');
        process.removeAllListeners('SIGINT');
        process.removeAllListeners('beforeExit');
    });

    // process.emit is typed with strict overloads per signal name; cast through
    // a loose-typed alias so tests can fire the events we just registered.
    // Must keep `this` bound to process or EventEmitter lookups crash.
    const emit = (event: string, ...args: unknown[]): boolean => (process.emit as unknown as (event: string, ...args: unknown[]) => boolean).call(process, event, ...args);

    it('captures uncaughtException via Client.captureException', () => {
        registerNodeGlobalHandlers();
        const err = new Error('boom');
        emit('uncaughtException', err, 'uncaughtException');
        expect(captureSpy).toHaveBeenCalledWith(err, { tags: { source: 'uncaughtException' } });
    });

    it('captures unhandledRejection via Client.captureException', () => {
        registerNodeGlobalHandlers();
        const err = new Error('async boom');
        emit('unhandledRejection', err, Promise.resolve());
        expect(captureSpy).toHaveBeenCalledWith(err, { tags: { source: 'unhandledRejection' } });
    });

    it('wraps string reasons in an Error', () => {
        registerNodeGlobalHandlers();
        emit('unhandledRejection', 'just a string', Promise.resolve());
        expect(captureSpy).toHaveBeenCalledTimes(1);
        const arg = captureSpy.mock.calls[0]![0] as Error;
        expect(arg).toBeInstanceOf(Error);
        expect(arg.message).toBe('just a string');
    });

    it('is idempotent — second call does not double-attach handlers', () => {
        registerNodeGlobalHandlers();
        const firstCount = process.listenerCount('uncaughtException');
        registerNodeGlobalHandlers();
        expect(process.listenerCount('uncaughtException')).toBe(firstCount);
    });

    it('invokes flush hook on SIGTERM with a timeout argument', () => {
        const flush = vi.fn().mockResolvedValue(undefined);
        registerNodeGlobalHandlers({ flush });
        emit('SIGTERM', 'SIGTERM');
        expect(flush).toHaveBeenCalledWith(2_000);
    });

    it('swallows errors from capture — does not propagate', () => {
        captureSpy.mockImplementation(() => {
            throw new Error('transport down');
        });
        registerNodeGlobalHandlers();
        expect(() => emit('uncaughtException', new Error('x'), 'uncaughtException')).not.toThrow();
    });
});
