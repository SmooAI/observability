/**
 * Minimal synchronous SHA-256 + HMAC-SHA256.
 *
 * Why hand-rolled rather than `node:crypto` or WebCrypto: `scrubString` is
 * synchronous and runs in **both** the browser and Node bundles. `node:crypto`
 * breaks the browser build, and WebCrypto's `subtle.sign` is async-only — it
 * cannot be called from a sync scrubber. This is the only sync primitive that
 * works in both runtimes without adding a dependency to a published SDK.
 *
 * Correctness is pinned by the RFC 4231 / FIPS 180-4 vectors in
 * `__tests__/hmac-sha256.test.ts`. Do not "optimize" this file without
 * re-running them.
 *
 * Not constant-time, and not intended for verifying MACs against attacker-
 * supplied values — the only consumer is `pii.ts`, which hashes values it
 * already holds.
 */

const K = /* @__PURE__ */ new Uint32Array([
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74,
    0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d,
    0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e,
    0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const BLOCK_BYTES = 64;

function rotr(x: number, n: number): number {
    return (x >>> n) | (x << (32 - n));
}

/** FIPS 180-4 SHA-256. Returns the 32-byte digest. */
export function sha256(message: Uint8Array): Uint8Array {
    const h = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);

    // Pad to a multiple of 64 bytes: 0x80, zeroes, then the 64-bit big-endian
    // bit length.
    const paddedLength = Math.ceil((message.length + 9) / BLOCK_BYTES) * BLOCK_BYTES;
    const padded = new Uint8Array(paddedLength);
    padded.set(message);
    padded[message.length] = 0x80;
    const view = new DataView(padded.buffer);
    const bitLength = message.length * 8;
    view.setUint32(paddedLength - 8, Math.floor(bitLength / 0x100000000), false);
    view.setUint32(paddedLength - 4, bitLength >>> 0, false);

    const w = new Uint32Array(64);
    for (let offset = 0; offset < paddedLength; offset += BLOCK_BYTES) {
        for (let i = 0; i < 16; i++) w[i] = view.getUint32(offset + i * 4, false);
        for (let i = 16; i < 64; i++) {
            const x = w[i - 15]!;
            const y = w[i - 2]!;
            const s0 = rotr(x, 7) ^ rotr(x, 18) ^ (x >>> 3);
            const s1 = rotr(y, 17) ^ rotr(y, 19) ^ (y >>> 10);
            w[i] = (w[i - 16]! + s0 + w[i - 7]! + s1) >>> 0;
        }

        let a = h[0]!;
        let b = h[1]!;
        let c = h[2]!;
        let d = h[3]!;
        let e = h[4]!;
        let f = h[5]!;
        let g = h[6]!;
        let hh = h[7]!;

        for (let i = 0; i < 64; i++) {
            const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            const ch = (e & f) ^ (~e & g);
            const temp1 = (hh + s1 + ch + K[i]! + w[i]!) >>> 0;
            const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            const maj = (a & b) ^ (a & c) ^ (b & c);
            const temp2 = (s0 + maj) >>> 0;

            hh = g;
            g = f;
            f = e;
            e = (d + temp1) >>> 0;
            d = c;
            c = b;
            b = a;
            a = (temp1 + temp2) >>> 0;
        }

        h[0] = (h[0]! + a) >>> 0;
        h[1] = (h[1]! + b) >>> 0;
        h[2] = (h[2]! + c) >>> 0;
        h[3] = (h[3]! + d) >>> 0;
        h[4] = (h[4]! + e) >>> 0;
        h[5] = (h[5]! + f) >>> 0;
        h[6] = (h[6]! + g) >>> 0;
        h[7] = (h[7]! + hh) >>> 0;
    }

    const digest = new Uint8Array(32);
    const digestView = new DataView(digest.buffer);
    for (let i = 0; i < 8; i++) digestView.setUint32(i * 4, h[i]!, false);
    return digest;
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
    const out = new Uint8Array(a.length + b.length);
    out.set(a);
    out.set(b, a.length);
    return out;
}

/** RFC 2104 HMAC-SHA256. Returns the 32-byte MAC. */
export function hmacSha256(key: Uint8Array, message: Uint8Array): Uint8Array {
    const block = new Uint8Array(BLOCK_BYTES);
    block.set(key.length > BLOCK_BYTES ? sha256(key) : key);

    const inner = new Uint8Array(BLOCK_BYTES);
    const outer = new Uint8Array(BLOCK_BYTES);
    for (let i = 0; i < BLOCK_BYTES; i++) {
        inner[i] = block[i]! ^ 0x36;
        outer[i] = block[i]! ^ 0x5c;
    }
    return sha256(concat(outer, sha256(concat(inner, message))));
}

/** Lowercase hex of `bytes`, truncated to `length` characters. */
export function toHex(bytes: Uint8Array, length = bytes.length * 2): string {
    let out = '';
    for (let i = 0; i < bytes.length && out.length < length; i++) {
        out += bytes[i]!.toString(16).padStart(2, '0');
    }
    return out.slice(0, length);
}
