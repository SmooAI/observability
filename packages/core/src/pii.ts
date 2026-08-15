/**
 * PII scrubbing — applied to message strings, breadcrumb messages, and headers
 * before transport. Mirrors `rust/observability/src/pii.rs`; the semantics are
 * identical across the five SDKs.
 *
 * Two classes, handled differently on purpose:
 *
 * - **Credentials** (`Bearer …`, `password=`, `token`/`api_key`/`secret=`,
 *   `sk-…`) are **dropped**. A hash of a live token is still a token oracle,
 *   and there is no correlation value in a secret.
 * - **Personal identifiers** (email, phone, street address) are **hashed**, not
 *   dropped: `a@b.com` → `[email:9f2a41c8]`. That keeps the one question worth
 *   asking — "are these two spans the same person?" — answerable while storing
 *   nothing reversible.
 *
 * The hash is **HMAC-SHA256, keyed**, not a bare digest: emails and phone
 * numbers are a small enumerable space that a rainbow table reverses in
 * seconds. The org id is mixed into the HMAC message, so identical PII hashes
 * **differently in different orgs**.
 *
 * **The key and the org salt are load-bearing and must not rotate casually.**
 * Rotating either silently breaks correlation with every previously stored
 * hash. Supply the key once at startup via `SMOOAI_OBSERVABILITY_PII_HASH_KEY`
 * (read by `bootstrapObservability`) or {@link setPiiHashKey} — the browser
 * bundle has no env, so it must call `setPiiHashKey` explicitly. **With no key
 * configured, personal identifiers are fully redacted** (`[email:redacted]`)
 * rather than hashed under a guessable one — fail safe, never fail open.
 *
 * Pattern matching never catches everything. It fails safe, which is the right
 * default for data we persist from end-user conversations; tenants can extend
 * in `beforeSend`.
 */

import { hmacSha256, toHex } from './hmac-sha256';

/**
 * Credentials. Matched FIRST so a personal identifier sitting inside a secret
 * (`token=a@b.com`) is dropped with the secret rather than surviving as a hash.
 */
const CREDENTIAL_PATTERNS: Array<{ re: RegExp; replacement: string | ((match: string) => string) }> = [
    { re: /Bearer\s+[A-Za-z0-9._-]+/gi, replacement: 'Bearer [redacted]' },
    { re: /\b(?:password|passwd|pwd)["']?\s*[:=]\s*["']?[^"'&\s]+/gi, replacement: 'password=[redacted]' },
    // Key-preserving: keep everything up to and including the `=`/`:`
    // separator, redact the rest.
    {
        re: /\b(?:token|api[-_]?key|apikey|secret)["']?\s*[:=]\s*["']?[^"'&\s]+/gi,
        replacement: (match: string) => match.replace(/^(.*?[:=]).*$/, '$1[redacted]'),
    },
    { re: /\bsk-[A-Za-z0-9]{20,}/g, replacement: 'sk-[redacted]' },
];

/**
 * The class of personal identifier a match represents. Drives both the visible
 * prefix in the output token and the normalization applied before hashing, so
 * `(415) 555-0142` and `415-555-0142` correlate.
 */
export type PiiKind = 'email' | 'phone' | 'address';

function normalize(kind: PiiKind, raw: string): string {
    switch (kind) {
        case 'email':
            return raw.trim().toLowerCase();
        case 'phone':
            // Digits only: formatting must not fork the hash.
            return raw.replace(/[^0-9]/g, '');
        case 'address':
            // Case-fold and collapse runs of whitespace.
            return raw.trim().replace(/\s+/g, ' ').toLowerCase();
    }
}

/**
 * Personal identifiers — hashed, not dropped. Order matters only in that these
 * all run after {@link CREDENTIAL_PATTERNS}.
 */
const PERSONAL_PATTERNS: Array<{ kind: PiiKind; re: RegExp }> = [
    { kind: 'email', re: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+\b/gi },
    // Phone: optional country code / area code, then the 3-4 local pair. A
    // separator is REQUIRED so bare digit runs (ids, timestamps, amounts) don't
    // get eaten.
    { kind: 'phone', re: /(?:\+\d{1,3}[ .-]?)?(?:\(\d{3}\)[ .-]?|\b\d{3}[ .-])?\b\d{3}[ .-]\d{4}\b/g },
    // US-style street address: house number, 1-3 words, a street suffix.
    {
        kind: 'address',
        re: /\b\d{1,6}\s+(?:[A-Za-z0-9.'-]+\s+){0,3}(?:street|st|avenue|ave|road|rd|boulevard|blvd|lane|ln|drive|dr|court|ct|way|terrace|ter|place|pl|circle|cir|highway|hwy|parkway|pkwy|square|sq)\b\.?/gi,
    },
];

const SENSITIVE_HEADERS = new Set(['authorization', 'cookie', 'set-cookie', 'x-api-key', 'x-auth-token']);

/**
 * Hex characters kept from the HMAC. Long enough that collisions are rare
 * across an org's traces, short enough to read in a span attribute.
 */
const HASH_HEX_LEN = 8;

const encoder = new TextEncoder();

let piiHashKey: Uint8Array | null = null;

/**
 * Install the process-wide HMAC key used to hash personal identifiers.
 * Idempotent and set-once: returns `false` if a key was already installed (the
 * existing key is kept — a mid-process rotation would silently fork every
 * correlation) or if `key` is empty. `bootstrapObservability` calls this from
 * `SMOOAI_OBSERVABILITY_PII_HASH_KEY`.
 */
export function setPiiHashKey(key: string | Uint8Array): boolean {
    const bytes = typeof key === 'string' ? encoder.encode(key) : key;
    if (bytes.length === 0 || piiHashKey !== null) return false;
    piiHashKey = bytes;
    return true;
}

/** Test seam — clears the set-once key so set-once itself can be asserted. */
export function _resetPiiHashKeyForTests(): void {
    piiHashKey = null;
}

/**
 * Hash one known-personal value into its scrubbed token — the same token
 * {@link scrubStringForOrg} would have written. This is how a UI search box
 * finds stored hashes: hash the typed term with the same org and match.
 *
 * Returns `[<kind>:redacted]` when no key is installed.
 */
export function piiToken(kind: PiiKind, raw: string, orgId: string): string {
    return tokenWithKey(kind, raw, orgId, piiHashKey);
}

function tokenWithKey(kind: PiiKind, raw: string, orgId: string, key: Uint8Array | null): string {
    if (!key || key.length === 0) return `[${kind}:redacted]`;
    // orgId in the HMAC message IS the per-org salt. The kind is in there too so
    // a phone and an address that normalize alike can't collide.
    // NUL separators (written as explicit escapes — a literal NUL in source is
    // invisible and formatter-fragile) so no field can impersonate a boundary.
    const message = encoder.encode(`${orgId}\u0000${kind}\u0000${normalize(kind, raw)}`);
    return `[${kind}:${toHex(hmacSha256(key, message), HASH_HEX_LEN)}]`;
}

/**
 * Scrub a free-form string with no org context — credentials dropped, personal
 * identifiers hashed under the empty org salt. Prefer {@link scrubStringForOrg}
 * wherever an org id is in hand, so hashes can't be correlated across tenants.
 */
export function scrubString(input: string): string {
    return scrubWithKey(input, '', piiHashKey);
}

/** Scrub a free-form string, salting personal-identifier hashes with `orgId`. */
export function scrubStringForOrg(input: string, orgId: string): string {
    return scrubWithKey(input, orgId, piiHashKey);
}

/** @internal Test seam — scrub with an explicit key instead of the global one. */
export function _scrubWithKey(input: string, orgId: string, key: Uint8Array | null): string {
    return scrubWithKey(input, orgId, key);
}

/** @internal Test seam — token with an explicit key instead of the global one. */
export function _tokenWithKey(kind: PiiKind, raw: string, orgId: string, key: Uint8Array | null): string {
    return tokenWithKey(kind, raw, orgId, key);
}

function scrubWithKey(input: string, orgId: string, key: Uint8Array | null): string {
    let out = input;
    for (const { re, replacement } of CREDENTIAL_PATTERNS) {
        out = typeof replacement === 'string' ? out.replace(re, replacement) : out.replace(re, (match: string) => replacement(match));
    }
    for (const { kind, re } of PERSONAL_PATTERNS) {
        out = out.replace(re, (match) => tokenWithKey(kind, match, orgId, key));
    }
    return out;
}

/**
 * Scrub a header map: sensitive header names are fully redacted, all other
 * values are run through {@link scrubString}.
 */
export function scrubHeaders(headers: Record<string, string> | undefined): Record<string, string> | undefined {
    return scrubHeadersForOrg(headers, '');
}

/** {@link scrubHeaders} with an org salt for the personal-identifier hashes. */
export function scrubHeadersForOrg(headers: Record<string, string> | undefined, orgId: string): Record<string, string> | undefined {
    if (!headers) return headers;
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(headers)) {
        out[k] = SENSITIVE_HEADERS.has(k.toLowerCase()) ? '[redacted]' : scrubStringForOrg(v, orgId);
    }
    return out;
}
