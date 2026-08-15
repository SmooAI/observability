import { beforeEach, describe, expect, it } from 'vitest';
import { _resetPiiHashKeyForTests, _scrubWithKey, _tokenWithKey, piiToken, scrubHeaders, scrubHeadersForOrg, scrubString, setPiiHashKey } from '../pii';

/**
 * Mirrors `rust/observability/src/pii.rs`'s test suite so parity across the
 * five SDKs is auditable case-by-case.
 *
 * The hashing tests drive `_scrubWithKey` / `_tokenWithKey` rather than
 * installing the process-wide key: `setPiiHashKey` is set-once and vitest runs
 * the file in one module instance, so a global write would make the suite
 * order-dependent.
 */

const enc = new TextEncoder();
const KEY = enc.encode('test-hmac-key-not-a-real-secret');
const OTHER_KEY = enc.encode('a-different-test-hmac-key');

const scrub = (input: string, org = 'org-1') => _scrubWithKey(input, org, KEY);

// ---- credentials: dropped, never hashed ----------------------------------

describe('scrubString — credentials', () => {
    it('redacts Bearer tokens', () => {
        expect(scrubString('Authorization: Bearer abc.def.ghi')).toBe('Authorization: Bearer [redacted]');
    });
    it('redacts password=', () => {
        expect(scrubString('?password=hunter2&x=1')).toBe('?password=[redacted]&x=1');
    });
    it('redacts token= while keeping the key', () => {
        const out = scrubString('api_key=supersecretvalue');
        expect(out).toBe('api_key=[redacted]');
        expect(out).not.toContain('supersecretvalue');
    });
    it('redacts sk-... API keys', () => {
        expect(scrubString('key=sk-AAAAAAAAAAAAAAAAAAAAAAAAAAAA')).toContain('sk-[redacted]');
    });

    it('drops credentials rather than hashing them', () => {
        // A live token must never become a correlatable handle — hashing one
        // still yields an oracle you can test candidate tokens against.
        const out = scrub('Bearer abc.def-ghi_123 password=hunter2 sk-ABCDEFGHIJKLMNOPQRSTUVWX');
        expect(out).toContain('Bearer [redacted]');
        expect(out).toContain('password=[redacted]');
        expect(out).toContain('sk-[redacted]');
        for (const shape of ['[token:', '[credential:', '[email:', '[phone:']) {
            expect(out).not.toContain(shape);
        }
        // And a personal identifier hiding inside a secret goes with it.
        const inSecret = scrub('token=a@b.com');
        expect(inSecret).not.toContain('a@b.com');
        expect(inSecret).not.toContain('[email:');
    });
});

// ---- personal identifiers: hashed, prefix preserved ----------------------

describe('scrubString — personal identifiers', () => {
    it('hashes emails keeping the type prefix in place', () => {
        const out = scrub('contact me at Alice@Example.com please');
        expect(out.toLowerCase()).not.toContain('alice@example.com');
        expect(out.startsWith('contact me at [email:')).toBe(true);
        expect(out.endsWith('] please')).toBe(true);
    });

    it.each(['555-0142', '(415) 555-0142', '+1 415-555-0142'])('hashes phone number %s', (raw) => {
        const out = scrub(`call ${raw} today`);
        expect(out).toContain('[phone:');
        expect(out).not.toContain('0142');
    });

    it('hashes street addresses', () => {
        const out = scrub('ship to 1600 Pennsylvania Ave, Washington');
        expect(out).toContain('[address:');
        expect(out).not.toContain('Pennsylvania');
    });

    it('is stable for the same value in the same org', () => {
        expect(scrub('a@b.com')).toBe(scrub('a@b.com'));
        // …and correlation survives formatting differences in phones.
        expect(scrub('(415) 555-0142')).toBe(scrub('415-555-0142'));
    });

    it('hashes the same value differently in a different org', () => {
        const a = scrub('a@b.com', 'org-1');
        const b = scrub('a@b.com', 'org-2');
        expect(a).not.toBe(b);
        expect(a.startsWith('[email:')).toBe(true);
        expect(b.startsWith('[email:')).toBe(true);
    });

    it('hashes differently under a different key', () => {
        expect(_scrubWithKey('a@b.com', 'org-1', KEY)).not.toBe(_scrubWithKey('a@b.com', 'org-1', OTHER_KEY));
    });

    it('redacts rather than hashing when no key is installed', () => {
        // Fail safe: an unkeyed digest of an email is rainbow-tabled instantly.
        const out = _scrubWithKey('a@b.com and 555-0142', 'org-1', null);
        expect(out).toContain('[email:redacted]');
        expect(out).toContain('[phone:redacted]');
        expect(out).not.toContain('a@b.com');
        expect(out).not.toContain('0142');
    });

    it('emits a short lowercase hex hash', () => {
        const token = _tokenWithKey('email', 'a@b.com', 'org-1', KEY);
        const hex = token.slice('[email:'.length, -1);
        expect(hex).toHaveLength(8);
        expect(hex).toMatch(/^[0-9a-f]{8}$/);
    });

    it('leaves ordinary numbers alone', () => {
        // The phone pattern requires a separator before the last 4 digits; ids,
        // versions, dates and amounts must survive intact.
        for (const value of ['took 1234 ms', 'version 1.2.3', '2026-08-15T12:00:00Z', 'total 1234.5678', 'trace 0af7651916cd43dd8448eb211c80319c']) {
            expect(scrub(value)).toBe(value);
        }
    });
});

// ---- the search seam ------------------------------------------------------

describe('piiToken', () => {
    beforeEach(() => {
        _resetPiiHashKeyForTests();
    });

    it('produces exactly what scrubbing wrote', () => {
        // The searchability contract: hashing a typed query term must produce
        // exactly the token stored in the span.
        const scrubbed = _scrubWithKey('mail a@b.com now', 'org-7', KEY);
        expect(scrubbed).toContain(_tokenWithKey('email', 'A@B.com ', 'org-7', KEY));
    });

    it('redacts when no key is installed', () => {
        expect(piiToken('email', 'a@b.com', 'org-1')).toBe('[email:redacted]');
    });
});

// ---- cross-SDK byte parity -----------------------------------------------

describe('cross-SDK parity vectors', () => {
    // Computed independently (python `hmac.new(key, org\0kind\0normalized,
    // "sha256")`) and asserted verbatim in all five SDKs. If any SDK's message
    // framing, normalization or truncation drifts, exactly one of these breaks.
    it.each([
        ['email', 'a@b.com', 'org-1', '[email:02ea437f]'],
        ['email', 'A@B.COM ', 'org-1', '[email:02ea437f]'],
        ['email', 'a@b.com', 'org-2', '[email:fd96f7dc]'],
        ['email', 'a@b.com', '', '[email:453b154f]'],
        ['phone', '(415) 555-0142', 'org-1', '[phone:415a9aea]'],
        ['phone', '415-555-0142', 'org-1', '[phone:415a9aea]'],
        ['address', '1600  Pennsylvania   Ave', 'org-1', '[address:c5351f4a]'],
    ] as const)('%s %s in %s', (kind, raw, org, expected) => {
        expect(_tokenWithKey(kind, raw, org, KEY)).toBe(expected);
    });
});

// ---- set-once key ---------------------------------------------------------

describe('setPiiHashKey', () => {
    beforeEach(() => {
        _resetPiiHashKeyForTests();
    });

    it('rejects an empty key and refuses rotation', () => {
        expect(setPiiHashKey('')).toBe(false);
        expect(setPiiHashKey('first-key')).toBe(true);
        expect(setPiiHashKey('second-key')).toBe(false);
        // The first key is what is still in force.
        expect(piiToken('email', 'a@b.com', 'o')).toBe(_tokenWithKey('email', 'a@b.com', 'o', enc.encode('first-key')));
        _resetPiiHashKeyForTests();
    });
});

// ---- headers --------------------------------------------------------------

describe('scrubHeaders', () => {
    it('redacts known sensitive headers', () => {
        const out = scrubHeaders({ authorization: 'Bearer abc', 'x-api-key': '12345', accept: 'application/json' })!;
        expect(out.authorization).toBe('[redacted]');
        expect(out['x-api-key']).toBe('[redacted]');
        expect(out.accept).toBe('application/json');
    });

    it('never leaks a raw email through a non-sensitive header', () => {
        const out = scrubHeadersForOrg({ 'X-Note': 'reply to a@b.com' }, 'org-1')!;
        expect(out['X-Note']).not.toContain('a@b.com');
        expect(out['X-Note']).toContain('[email:');
    });
});
