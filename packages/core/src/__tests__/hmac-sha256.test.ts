import { describe, expect, it } from 'vitest';
import { hmacSha256, sha256, toHex } from '../hmac-sha256';

/**
 * The hand-rolled sync primitive `pii.ts` depends on. These are the published
 * FIPS 180-4 / RFC 4231 vectors — if this file is green the implementation is
 * the real algorithm, not something that merely looks like a hash.
 */

const enc = new TextEncoder();
const hex = (b: Uint8Array) => toHex(b);

describe('sha256', () => {
    it('matches the FIPS 180-4 vectors', () => {
        expect(hex(sha256(enc.encode('')))).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
        expect(hex(sha256(enc.encode('abc')))).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
        expect(hex(sha256(enc.encode('abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq')))).toBe(
            '248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1',
        );
    });

    it('handles the 55/56/64-byte padding boundaries', () => {
        // 56 bytes is where the length field no longer fits in the same block —
        // the classic off-by-one in a hand-rolled pad.
        expect(hex(sha256(enc.encode('a'.repeat(55))))).toBe('9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318');
        expect(hex(sha256(enc.encode('a'.repeat(56))))).toBe('b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a');
        expect(hex(sha256(enc.encode('a'.repeat(64))))).toBe('ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb');
    });

    it('hashes a million a-s (FIPS long vector)', () => {
        expect(hex(sha256(enc.encode('a'.repeat(1_000_000))))).toBe('cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0');
    });
});

describe('hmacSha256', () => {
    it('matches RFC 4231 test case 1', () => {
        const key = new Uint8Array(20).fill(0x0b);
        expect(hex(hmacSha256(key, enc.encode('Hi There')))).toBe('b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7');
    });

    it('matches RFC 4231 test case 2', () => {
        expect(hex(hmacSha256(enc.encode('Jefe'), enc.encode('what do ya want for nothing?')))).toBe(
            '5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843',
        );
    });

    it('matches RFC 4231 test case 3', () => {
        const key = new Uint8Array(20).fill(0xaa);
        const data = new Uint8Array(50).fill(0xdd);
        expect(hex(hmacSha256(key, data))).toBe('773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe');
    });

    it('matches RFC 4231 test case 6 — key longer than one block', () => {
        const key = new Uint8Array(131).fill(0xaa);
        expect(hex(hmacSha256(key, enc.encode('Test Using Larger Than Block-Size Key - Hash Key First')))).toBe(
            '60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54',
        );
    });
});

describe('toHex', () => {
    it('truncates to the requested length', () => {
        const bytes = new Uint8Array([0x00, 0xab, 0xff, 0x10]);
        expect(toHex(bytes)).toBe('00abff10');
        expect(toHex(bytes, 4)).toBe('00ab');
        // Leading zeroes must survive — a naive toString(16) drops them.
        expect(toHex(new Uint8Array([0x01, 0x02]))).toBe('0102');
    });
});
