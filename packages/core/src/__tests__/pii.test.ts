import { describe, expect, it } from 'vitest';
import { scrubHeaders, scrubString } from '../pii';

describe('scrubString', () => {
    it('redacts Bearer tokens', () => {
        expect(scrubString('Authorization: Bearer abc.def.ghi')).toBe('Authorization: Bearer [redacted]');
    });
    it('redacts password=', () => {
        expect(scrubString('?password=hunter2&x=1')).toBe('?password=[redacted]&x=1');
    });
    it('redacts sk-... API keys', () => {
        expect(scrubString('key=sk-AAAAAAAAAAAAAAAAAAAAAAAAAAAA')).toContain('sk-[redacted]');
    });
});

describe('scrubHeaders', () => {
    it('redacts known sensitive headers', () => {
        const out = scrubHeaders({ authorization: 'Bearer abc', 'x-api-key': '12345', accept: 'application/json' })!;
        expect(out.authorization).toBe('[redacted]');
        expect(out['x-api-key']).toBe('[redacted]');
        expect(out.accept).toBe('application/json');
    });
});
