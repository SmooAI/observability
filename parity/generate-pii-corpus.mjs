#!/usr/bin/env node
/**
 * Regenerates `parity/pii-corpus.json`.
 *
 * The token derivation below is written against the SPEC (see
 * `parity/PII-README.md`), not lifted from any SDK — `node:crypto` computes the
 * HMAC, and the normalization is re-expressed here. Before writing anything it
 * self-checks against the seven tuples that were published (as hand-copied
 * literals) in all five SDKs' test files, so a generator bug cannot silently
 * redefine the contract: if this file disagrees with what shipped, it refuses
 * to write.
 *
 * Usage: node parity/generate-pii-corpus.mjs
 */
import { createHmac } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The test key. NOT a secret — it exists so five languages can agree on bytes. */
const KEY = 'test-hmac-key-not-a-real-secret';

/** A second key, to pin that the token is genuinely keyed. */
const OTHER_KEY = 'a-different-test-hmac-key';

/** Hex characters kept from the HMAC. */
const HASH_HEX_LEN = 8;

function normalize(kind, raw) {
    switch (kind) {
        case 'email':
            return raw.trim().toLowerCase();
        case 'phone':
            // Digits only: formatting must not fork the hash.
            return [...raw].filter((c) => c >= '0' && c <= '9').join('');
        case 'address':
            // Case-fold and collapse runs of whitespace.
            return raw.split(/\s+/u).filter(Boolean).join(' ').toLowerCase();
        default:
            throw new Error(`unknown kind ${kind}`);
    }
}

function token(kind, raw, orgId, key) {
    // org_id is the per-org salt; the kind is in the message too, so a phone and
    // an address that normalize alike cannot collide. NUL-delimited so
    // ("a", "bc") and ("ab", "c") cannot frame to the same bytes.
    const message = Buffer.concat([
        Buffer.from(orgId, 'utf8'),
        Buffer.from([0]),
        Buffer.from(kind, 'utf8'),
        Buffer.from([0]),
        Buffer.from(normalize(kind, raw), 'utf8'),
    ]);
    const mac = createHmac('sha256', Buffer.from(key, 'utf8')).update(message).digest('hex');
    return `[${kind}:${mac.slice(0, HASH_HEX_LEN)}]`;
}

/**
 * The seven tuples that shipped as hand-copied literals in every SDK's test
 * file before this corpus existed. They are the self-check: if the derivation
 * above disagrees with any of them, this generator is wrong, not the SDKs.
 */
const PUBLISHED = [
    ['email', 'a@b.com', 'org-1', '[email:02ea437f]'],
    ['email', 'A@B.COM ', 'org-1', '[email:02ea437f]'],
    ['email', 'a@b.com', 'org-2', '[email:fd96f7dc]'],
    ['email', 'a@b.com', '', '[email:453b154f]'],
    ['phone', '(415) 555-0142', 'org-1', '[phone:415a9aea]'],
    ['phone', '415-555-0142', 'org-1', '[phone:415a9aea]'],
    ['address', '1600  Pennsylvania   Ave', 'org-1', '[address:c5351f4a]'],
];

for (const [kind, raw, orgId, expected] of PUBLISHED) {
    const got = token(kind, raw, orgId, KEY);
    if (got !== expected) {
        throw new Error(`self-check failed: token(${kind}, ${JSON.stringify(raw)}, ${JSON.stringify(orgId)}) = ${got}, published ${expected}`);
    }
}

/** `[kind, raw, orgId, why]` — `why` is documentation, not part of the assertion. */
const CASES = [
    ['email', 'a@b.com', 'org-1', 'the baseline vector'],
    ['email', 'A@B.COM ', 'org-1', 'case-folded and trimmed — same hash as the baseline'],
    ['email', '  a@b.com  ', 'org-1', 'leading whitespace trims too, not just trailing'],
    ['email', 'a@b.com', 'org-2', 'the org id is the salt — a different org hashes differently'],
    ['email', 'a@b.com', '', 'empty org is a real salt value, not "skip the salt"'],
    ['email', 'a+tag@b.com', 'org-1', 'plus-addressing is NOT stripped — a+tag@ is a different person than a@'],
    ['email', 'Ünïcode@exämple.com', 'org-1', 'lowercasing is Unicode-aware and the message is UTF-8'],
    ['phone', '(415) 555-0142', 'org-1', 'parens and spaces are formatting'],
    ['phone', '415-555-0142', 'org-1', 'so are dashes — same hash'],
    ['phone', '415.555.0142', 'org-1', 'and dots — same hash'],
    ['phone', '+1 415-555-0142', 'org-1', 'a country code is DIGITS, so it does change the hash'],
    ['address', '1600  Pennsylvania   Ave', 'org-1', 'runs of spaces collapse to one'],
    ['address', '1600 PENNSYLVANIA AVE', 'org-1', 'case-folded — same hash'],
    ['address', '1600\tPennsylvania\nAve', 'org-1', 'tabs and newlines are whitespace too, not just spaces'],
    ['address', '1600 Pennsylvania Ave', 'org-2', 'salted by org like every other kind'],
];

const corpus = {
    $schema: './PII-README.md',
    version: 1,
    adr: 'ADR-097',
    description: 'Golden PII-token vectors every @smooai/observability SDK (TS, Rust, Python, Go, .NET) must reproduce. See parity/PII-README.md.',
    hash: {
        algorithm: 'HMAC-SHA256',
        key: KEY,
        otherKey: OTHER_KEY,
        message: 'utf8(orgId) || 0x00 || utf8(kind) || 0x00 || utf8(normalize(kind, raw))',
        output: `"[" + kind + ":" + first ${HASH_HEX_LEN} lowercase hex chars of the MAC + "]"`,
        normalize: {
            email: 'trim, then lowercase',
            phone: 'keep ASCII digits only',
            address: 'collapse runs of whitespace to one space, then lowercase',
        },
        noKey: 'with no key installed the token is "[" + kind + ":redacted]" — fail safe, never a guessable hash',
    },
    /** kind + raw + orgId, hashed under `hash.key`. */
    tokenWithKey: CASES.map(([kind, raw, orgId, why]) => ({ kind, raw, orgId, expected: token(kind, raw, orgId, KEY), why })),
    /** The same inputs under a DIFFERENT key must produce different tokens. */
    tokenWithOtherKey: CASES.slice(0, 4).map(([kind, raw, orgId]) => ({ kind, raw, orgId, expected: token(kind, raw, orgId, OTHER_KEY) })),
    /** No key installed → redaction, never a hash under a guessable key. */
    tokenWithoutKey: [
        { kind: 'email', raw: 'a@b.com', orgId: 'org-1', expected: '[email:redacted]' },
        { kind: 'phone', raw: '415-555-0142', orgId: 'org-1', expected: '[phone:redacted]' },
        { kind: 'address', raw: '1600 Pennsylvania Ave', orgId: 'org-1', expected: '[address:redacted]' },
    ],
};

const out = join(dirname(fileURLToPath(import.meta.url)), 'pii-corpus.json');
writeFileSync(out, `${JSON.stringify(corpus, null, 4)}\n`, 'utf8');
console.log(`wrote ${out}: ${corpus.tokenWithKey.length} + ${corpus.tokenWithOtherKey.length} + ${corpus.tokenWithoutKey.length} vectors`);
