#!/usr/bin/env node
/**
 * Regenerates `parity/sampling-corpus.json` (ADR-097 §4).
 *
 * Run: `node parity/generate-corpus.mjs`
 *
 * The FNV-1a implementation here is written independently of the SDK's (Buffer
 * bytes rather than TextEncoder, no shared code) ON PURPOSE — the corpus is
 * only evidence if its expected values are not produced by the implementation
 * it is used to test. It is checked against the published FNV-1a-32 vectors
 * before it emits anything.
 *
 * Non-hash expectations (levels, traceparent, settings fallback) are authored
 * by hand below and derived from the spec, not from the code.
 */
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const fnv1a32 = (s) => {
    let h = 0x811c9dc5;
    for (const b of Buffer.from(s, 'utf8')) {
        h = (h ^ b) >>> 0;
        // Wrapping 32-bit multiply via 16-bit halves. `h * prime` would exceed
        // float64's 53-bit exact range, so masking the product is silently wrong.
        const lo = h & 0xffff;
        const hi = h >>> 16;
        h = ((lo * 0x01000193 + (((hi * 0x01000193) & 0xffff) << 16)) & 0xffffffff) >>> 0;
    }
    return h >>> 0;
};

// Published FNV-1a-32 vectors. If these ever fail, nothing below is trustworthy.
for (const [input, expected] of [
    ['', 0x811c9dc5],
    ['a', 0xe40c292c],
    ['foobar', 0xbf9cf968],
]) {
    if (fnv1a32(input) !== expected) throw new Error(`FNV-1a self-check failed for ${JSON.stringify(input)}`);
}

// Note: `((h * prime) & 0xffffffff)` loses precision above 2^53 in float64 —
// verify the wrapping multiply agrees with Math.imul, which is exact.
for (const s of ['', 'a', 'foobar', 'session-abcdef', '🎉', 'x'.repeat(64)]) {
    let h = 0x811c9dc5;
    for (const b of Buffer.from(s, 'utf8')) h = Math.imul((h ^ b) >>> 0, 0x01000193) >>> 0;
    if (h !== fnv1a32(s)) throw new Error(`wrapping-multiply mismatch for ${JSON.stringify(s)}`);
}

const decide = (id, ratio) => {
    if (!Number.isFinite(ratio)) return true;
    if (ratio <= 0) return false;
    if (ratio >= 1) return true;
    return fnv1a32(id) / 4294967296 < ratio;
};

const ids = [
    '', // empty string — must not throw, must be deterministic
    'a',
    'foobar',
    'session-00000000-0000-4000-8000-000000000000',
    '4bf92f3577b34da6a3ce929d0e0e4736', // a real-looking W3C trace id
    'user@example.com',
    '🎉-emoji-session', // multi-byte UTF-8: byte-wise hashing, not char-wise
    'Ünïcødé-sessión',
    '日本語セッション',
    'x'.repeat(256),
    ' leading-and-trailing ', // whitespace is significant — not trimmed
];

const ratios = [0, 0.01, 0.1, 0.25, 0.5, 0.9, 0.99, 1];

const sampleDecisionVectors = [];
for (const id of ids) {
    for (const ratio of ratios) {
        sampleDecisionVectors.push({ id, ratio, hash: fnv1a32(id), expected: decide(id, ratio) });
    }
}

// Ids whose hash lands within 1e-4 of the 0.5 threshold — the cases where a
// sloppy port (signed int, char-wise hashing, <= instead of <) diverges.
const nearThreshold = [];
for (let i = 0; nearThreshold.length < 6 && i < 500000; i++) {
    const id = `near-${i}`;
    const p = fnv1a32(id) / 4294967296;
    if (Math.abs(p - 0.5) < 1e-4) {
        nearThreshold.push({ id, ratio: 0.5, hash: fnv1a32(id), position: p, expected: decide(id, 0.5) });
    }
}
if (nearThreshold.length < 6) throw new Error('could not find enough near-threshold ids');

// Non-finite ratios fail OPEN (never silently dark). Encoded as strings because
// JSON has no NaN/Infinity literal; a porter maps these to their float values.
const nonFiniteRatioVectors = [
    { id: 'session-1', ratio: 'NaN', expected: true },
    { id: 'session-1', ratio: 'Infinity', expected: true },
    { id: 'session-1', ratio: '-Infinity', expected: true },
];

// ADR-096: the error-rate query is `level IN ('ERROR','FATAL')` and is
// CASE-SENSITIVE. Lowercase silently makes every error a non-error.
const levelNormalization = [
    { input: 'trace', expected: 'TRACE' },
    { input: 'verbose', expected: 'TRACE' },
    { input: 'debug', expected: 'DEBUG' },
    { input: 'DEBUG', expected: 'DEBUG' },
    { input: 'info', expected: 'INFO' },
    { input: 'Information', expected: 'INFO' },
    { input: 'log', expected: 'INFO' },
    { input: 'notice', expected: 'INFO' },
    { input: 'warn', expected: 'WARN' },
    { input: 'warning', expected: 'WARN' },
    { input: 'WARNING', expected: 'WARN' },
    { input: 'error', expected: 'ERROR' },
    { input: 'Error', expected: 'ERROR' },
    { input: 'ERROR', expected: 'ERROR' },
    { input: 'err', expected: 'ERROR' },
    { input: 'fatal', expected: 'FATAL' },
    { input: 'critical', expected: 'FATAL' },
    { input: 'crit', expected: 'FATAL' },
    { input: 'panic', expected: 'FATAL' },
    { input: 'emergency', expected: 'FATAL' },
    { input: '  Warn  ', expected: 'WARN' }, // surrounding whitespace tolerated
    { input: 'bogus', expected: 'INFO' }, // unknown → INFO, never ERROR
    { input: '', expected: 'INFO' },
];

const TRACE_ID = '4bf92f3577b34da6a3ce929d0e0e4736';
const SPAN_ID = '00f067aa0ba902b7';

const traceparentParse = [
    { input: `00-${TRACE_ID}-${SPAN_ID}-01`, expected: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 1, sampled: true } },
    { input: `00-${TRACE_ID}-${SPAN_ID}-00`, expected: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 0, sampled: false } },
    // Only bit 0 is the sampled flag; other bits are carried, not interpreted.
    { input: `00-${TRACE_ID}-${SPAN_ID}-03`, expected: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 3, sampled: true } },
    { input: `00-${TRACE_ID}-${SPAN_ID}-fe`, expected: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 254, sampled: false } },
    { input: `01-${TRACE_ID}-${SPAN_ID}-01`, expected: null, why: 'wrong version — strict parser accepts only 00' },
    { input: `ff-${TRACE_ID}-${SPAN_ID}-01`, expected: null, why: 'version ff is forbidden by the W3C spec' },
    { input: `00-${'0'.repeat(32)}-${SPAN_ID}-01`, expected: null, why: 'all-zero trace id is invalid' },
    { input: `00-${TRACE_ID}-${'0'.repeat(16)}-01`, expected: null, why: 'all-zero span id is invalid' },
    { input: `00-${TRACE_ID.toUpperCase()}-${SPAN_ID}-01`, expected: null, why: 'hex must be lowercase' },
    { input: `00-${TRACE_ID.slice(0, 31)}-${SPAN_ID}-01`, expected: null, why: 'trace id too short' },
    { input: `00-${TRACE_ID}-${SPAN_ID}-1`, expected: null, why: 'flags must be exactly two hex digits' },
    { input: `00-${TRACE_ID}-${SPAN_ID}`, expected: null, why: 'too few fields' },
    { input: `00-${TRACE_ID}-${SPAN_ID}-01-extra`, expected: null, why: 'too many fields for version 00' },
    { input: `00-${TRACE_ID}-zzzzzzzzzzzzzzzz-01`, expected: null, why: 'non-hex span id' },
    { input: '', expected: null, why: 'empty header' },
    { input: 'garbage', expected: null, why: 'not a traceparent' },
];

const traceparentFormat = [
    { input: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 1 }, expected: `00-${TRACE_ID}-${SPAN_ID}-01` },
    { input: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 0 }, expected: `00-${TRACE_ID}-${SPAN_ID}-00` },
    { input: { traceId: TRACE_ID, spanId: SPAN_ID, sampled: true }, expected: `00-${TRACE_ID}-${SPAN_ID}-01` },
    { input: { traceId: TRACE_ID, spanId: SPAN_ID, sampled: false }, expected: `00-${TRACE_ID}-${SPAN_ID}-00` },
    { input: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 255 }, expected: `00-${TRACE_ID}-${SPAN_ID}-ff` },
    { input: { traceId: '0'.repeat(32), spanId: SPAN_ID, flags: 1 }, expected: null, why: 'format must never emit what parse rejects' },
    { input: { traceId: TRACE_ID, spanId: 'short', flags: 1 }, expected: null, why: 'invalid span id' },
    { input: { traceId: TRACE_ID, spanId: SPAN_ID, flags: 256 }, expected: null, why: 'flags out of byte range' },
];

const D = { enabled: true, browserLogSamplingRatio: 1, minimumLogLevel: 'INFO', traceSamplingRatio: 0.1 };
const K = {
    enabled: 'observabilityEnabled',
    ratio: 'observabilityBrowserLogSamplingRatio',
    level: 'observabilityMinimumLogLevel',
    trace: 'observabilityTraceSamplingRatio',
};

const settingsResolution = [
    { input: null, expected: D, why: 'config unreachable → compiled-in ADR-010 defaults, never all-out' },
    { input: 'not-an-object', expected: D, why: 'malformed payload → defaults' },
    { input: [], expected: D, why: 'array payload → defaults' },
    { input: {}, expected: D, why: 'empty payload → defaults' },
    { input: { unrelated: 'key' }, expected: D, why: 'unknown keys ignored' },
    {
        input: { [K.enabled]: false, [K.ratio]: 0.25, [K.level]: 'warn', [K.trace]: 0.5 },
        expected: { enabled: false, browserLogSamplingRatio: 0.25, minimumLogLevel: 'WARN', traceSamplingRatio: 0.5 },
    },
    {
        input: { [K.enabled]: 'false', [K.ratio]: '0.25', [K.trace]: '1' },
        expected: { enabled: false, browserLogSamplingRatio: 0.25, minimumLogLevel: 'INFO', traceSamplingRatio: 1 },
        why: 'public config may round-trip values as strings',
    },
    { input: { [K.ratio]: 1.5 }, expected: { ...D, browserLogSamplingRatio: 1 }, why: '>1 clamps to 1' },
    { input: { [K.ratio]: -1 }, expected: { ...D, browserLogSamplingRatio: 0 }, why: 'negative clamps to 0' },
    { input: { [K.ratio]: 'NaN' }, expected: D, why: 'NaN is malformed → default, NOT 0' },
    { input: { [K.ratio]: 'banana' }, expected: D, why: 'unparseable → default' },
    { input: { [K.ratio]: null }, expected: D, why: 'null → default' },
    { input: { [K.ratio]: true }, expected: D, why: 'wrong type → default' },
    { input: { [K.level]: 'bogus' }, expected: D, why: "typo'd level → default floor, not INFO-by-coincidence" },
    { input: { [K.level]: 'FATAL' }, expected: { ...D, minimumLogLevel: 'FATAL' } },
    { input: { [K.enabled]: 'yes' }, expected: D, why: 'only true/false strings coerce' },
];

// shouldEmitLog: precedence is kill switch → min level → WARN+ always →
// trace decision → session decision.
const S = (over) => ({ enabled: true, minimumLevel: 'INFO', logSamplingRatio: 1, ...over });
const OUT_ID = 'session-out'; // hashes to 0.5758 — sampled OUT at ratio 0.5
const outIsOut = decide(OUT_ID, 0.5) === false;
if (!outIsOut) throw new Error(`${OUT_ID} must hash sampled-out at ratio 0.5`);

const shouldEmitLog = [
    { input: { level: 'error', sessionId: OUT_ID, ...S({ enabled: false }) }, expected: false, why: 'kill switch beats everything' },
    { input: { level: 'debug', sessionId: 'any', ...S({}) }, expected: false, why: 'below minimum level' },
    { input: { level: 'error', sessionId: OUT_ID, ...S({ logSamplingRatio: 0 }) }, expected: true, why: 'errors are always 100%' },
    { input: { level: 'warning', sessionId: OUT_ID, ...S({ logSamplingRatio: 0 }) }, expected: true, why: 'warnings are always 100%' },
    { input: { level: 'fatal', sessionId: OUT_ID, ...S({ logSamplingRatio: 0 }) }, expected: true },
    { input: { level: 'error', sessionId: OUT_ID, ...S({ minimumLevel: 'FATAL' }) }, expected: false, why: 'min level outranks always-on' },
    { input: { level: 'info', sessionId: OUT_ID, ...S({ logSamplingRatio: 0.5 }) }, expected: false, why: 'session sampled out' },
    { input: { level: 'info', sessionId: OUT_ID, traceSampled: true, ...S({ logSamplingRatio: 0 }) }, expected: true, why: 'trace decision wins' },
    { input: { level: 'info', sessionId: OUT_ID, traceSampled: false, ...S({ logSamplingRatio: 1 }) }, expected: false, why: 'trace decision wins' },
    { input: { level: 'info', sessionId: 'anything', ...S({}) }, expected: true, why: 'ratio 1.0 never drops' },
];

const corpus = {
    $schema: './README.md',
    version: 1,
    adr: 'ADR-097',
    description: 'Golden vectors every @smooai/observability SDK (TS, Rust, Python, Go, .NET) must reproduce. See parity/README.md.',
    hash: {
        algorithm: 'FNV-1a',
        bits: 32,
        offsetBasis: '0x811c9dc5',
        prime: '0x01000193',
        input: 'UTF-8 bytes of the id, folded one byte at a time (no endianness)',
        decision: 'ratio<=0 -> false; ratio>=1 -> true; non-finite ratio -> true (fail open); else (hash / 2^32) < ratio',
        note: 'hash is UNSIGNED 32-bit; divide by 4294967296.0 in binary64 — exact, no rounding',
    },
    sampleDecision: sampleDecisionVectors,
    sampleDecisionNearThreshold: nearThreshold,
    sampleDecisionNonFiniteRatio: nonFiniteRatioVectors,
    levelNormalization,
    traceparentParse,
    traceparentFormat,
    settingsResolution,
    shouldEmitLog,
};

const out = join(dirname(fileURLToPath(import.meta.url)), 'sampling-corpus.json');
writeFileSync(out, JSON.stringify(corpus, null, 4) + '\n');
const count =
    sampleDecisionVectors.length +
    nearThreshold.length +
    nonFiniteRatioVectors.length +
    levelNormalization.length +
    traceparentParse.length +
    traceparentFormat.length +
    settingsResolution.length +
    shouldEmitLog.length;
console.log(`wrote ${out} — ${count} vectors`);
