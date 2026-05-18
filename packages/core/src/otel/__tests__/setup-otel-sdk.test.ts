import { afterEach, describe, expect, it } from 'vitest';
import { _resetOtelSdkForTests, setupOtelSdk } from '../setup-otel-sdk';

describe('setupOtelSdk', () => {
    afterEach(() => {
        _resetOtelSdkForTests();
    });

    it('returns a handle with sdk, flush, shutdown', () => {
        const handle = setupOtelSdk({ serviceName: 'test', skipStart: true });
        expect(handle.sdk).toBeDefined();
        expect(typeof handle.flush).toBe('function');
        expect(typeof handle.shutdown).toBe('function');
    });

    it('is idempotent — second call returns the same handle', () => {
        const a = setupOtelSdk({ serviceName: 'test', skipStart: true });
        const b = setupOtelSdk({ serviceName: 'test', skipStart: true });
        expect(a).toBe(b);
    });

    it('shutdown clears the install guard so a new init returns a new handle', async () => {
        const a = setupOtelSdk({ serviceName: 'test', skipStart: true });
        await a.shutdown();
        const b = setupOtelSdk({ serviceName: 'test', skipStart: true });
        expect(b).not.toBe(a);
    });

    it('flush resolves within the timeout even when exporter is silent', async () => {
        const handle = setupOtelSdk({ serviceName: 'test', skipStart: true });
        const start = Date.now();
        await handle.flush(50);
        const elapsed = Date.now() - start;
        // Allow generous slack for CI scheduler jitter.
        expect(elapsed).toBeLessThan(500);
    });

    it('accepts disableAutoInstrumentations without crashing', () => {
        const handle = setupOtelSdk({ serviceName: 'test', skipStart: true, disableAutoInstrumentations: true });
        expect(handle.sdk).toBeDefined();
    });
});
