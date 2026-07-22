/**
 * ADR-097 — session-scoped sampling.
 *
 * THE CORE RULE: the sampling decision is made ONCE per session (or per trace,
 * where one exists) and applies to EVERY log line under it. The invariant this
 * buys, stated as an acceptance test: **any trace you can open has 100% of its
 * log lines**. Never a partial view.
 *
 * This file is the TypeScript reference implementation (ADR-009: the other four
 * SDKs are hand-written parity ports, gated on `parity/sampling-corpus.json`).
 *
 * ─────────────────────────────────────────────────────────────────────────────
 * THE HASH — everything a porter needs, in one place
 * ─────────────────────────────────────────────────────────────────────────────
 *
 * Algorithm: **FNV-1a, 32-bit**, over the **UTF-8 bytes** of the id.
 *
 *     h = 0x811c9dc5                      // FNV offset basis, 32-bit
 *     for each byte b of utf8(id):
 *         h = h XOR b                     // XOR first — this is FNV-*1a*
 *         h = (h * 0x01000193) mod 2^32   // FNV prime, wrapping 32-bit multiply
 *
 * `h` is an **unsigned** 32-bit integer. There is no endianness in the
 * computation itself (it is a byte-at-a-time fold, not a word load), so no
 * byte-order choice can diverge between languages. Languages with signed-only
 * 32-bit ints (Java/C#/Go int32) must reinterpret as unsigned before dividing.
 *
 * Decision:
 *
 *     if ratio is not finite  -> IN   (fail-open; see below)
 *     if ratio <= 0.0         -> OUT  (exact, no float compare)
 *     if ratio >= 1.0         -> IN   (exact, no float compare)
 *     else                    -> (h / 2^32) < ratio
 *
 * `h / 2^32` is exact in IEEE-754 binary64: `h` is < 2^32 so it needs at most
 * 32 mantissa bits, and 2^32 is a power of two, so the division is a pure
 * exponent adjustment with zero rounding. Every language that divides a
 * u32 by `4294967296.0` in binary64 gets the identical double. The comparison
 * is **strict less-than**.
 *
 * The 0.0 / 1.0 branches are taken *before* any float math specifically so that
 * ratio 1.0 can never drop a session and ratio 0.0 can never keep one, whatever
 * the hash happens to be.
 *
 * Non-finite ratio (NaN/Inf) fails **open**, not closed: `x < NaN` is false in
 * every IEEE-754 language, which would silently sample everything out — the
 * exact "telemetry goes quiet when config hiccups" failure ADR-097 forbids.
 *
 * Verified against the published FNV-1a-32 vectors (see `sampling.test.ts`):
 * "" -> 0x811c9dc5, "a" -> 0xe40c292c, "foobar" -> 0xbf9cf968.
 */

const FNV_OFFSET_BASIS_32 = 0x811c9dc5;
const FNV_PRIME_32 = 0x01000193;
const TWO_POW_32 = 4294967296;

const utf8 = new TextEncoder();

/** FNV-1a 32-bit over the UTF-8 bytes of `input`. Returns an unsigned 32-bit int. */
export function fnv1a32(input: string): number {
    let h = FNV_OFFSET_BASIS_32;
    const bytes = utf8.encode(input);
    for (let i = 0; i < bytes.length; i++) {
        h ^= bytes[i]!;
        // Math.imul does the wrapping 32-bit multiply; >>> 0 reinterprets as unsigned.
        h = Math.imul(h, FNV_PRIME_32) >>> 0;
    }
    return h >>> 0;
}

/**
 * The one sampling primitive. Deterministic, stable for the lifetime of an id,
 * and byte-identically reproducible in Rust/Python/Go/.NET (see header).
 *
 * @param id    session id, or trace id where a trace exists
 * @param ratio 0.0 (never) .. 1.0 (always)
 */
export function sampleDecision(id: string, ratio: number): boolean {
    if (!Number.isFinite(ratio)) return true; // fail open, never silently dark
    if (ratio <= 0) return false;
    if (ratio >= 1) return true;
    return fnv1a32(id) / TWO_POW_32 < ratio;
}

/**
 * Canonical log levels, uppercase.
 *
 * ADR-096's error-rate query is `level IN ('ERROR','FATAL')` and ClickHouse
 * string comparison is CASE-SENSITIVE — emitting `"error"` silently makes every
 * error a non-error. Normalization is therefore not cosmetic.
 */
export const LEVELS = ['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR', 'FATAL'] as const;
export type CanonicalLevel = (typeof LEVELS)[number];

/** Ordering used by the minimum-level filter. Matches OTel severity ordering. */
const LEVEL_RANK: Record<CanonicalLevel, number> = { TRACE: 1, DEBUG: 5, INFO: 9, WARN: 13, ERROR: 17, FATAL: 21 };

/**
 * Aliases every SDK must accept. Anything not listed normalizes to INFO —
 * fail-safe: an unrecognised level must never cause a drop, and must never be
 * promoted into ERROR (which would corrupt the error rate).
 */
const LEVEL_ALIASES: Record<string, CanonicalLevel> = {
    trace: 'TRACE',
    verbose: 'TRACE',
    debug: 'DEBUG',
    info: 'INFO',
    information: 'INFO',
    log: 'INFO',
    notice: 'INFO',
    warn: 'WARN',
    warning: 'WARN',
    error: 'ERROR',
    err: 'ERROR',
    fatal: 'FATAL',
    critical: 'FATAL',
    crit: 'FATAL',
    emergency: 'FATAL',
    panic: 'FATAL',
};

/** Strict parse: a known level spelling, or null. */
export function parseLevel(level: string): CanonicalLevel | null {
    return LEVEL_ALIASES[level.trim().toLowerCase()] ?? null;
}

/** Normalize any level spelling to the canonical UPPERCASE form. */
export function normalizeLevel(level: string): CanonicalLevel {
    return parseLevel(level) ?? 'INFO';
}

/** True when `level` is at or above `minimum`. */
export function meetsMinimumLevel(level: CanonicalLevel, minimum: CanonicalLevel): boolean {
    return LEVEL_RANK[level] >= LEVEL_RANK[minimum];
}

export interface LogSamplingInput {
    /** Level as emitted by the caller; normalized internally. */
    level: string;
    /** Stable per-page session id. Used when no trace context exists. */
    sessionId: string;
    /**
     * The trace's own sampling decision, when a trace context exists. Where a
     * trace exists its decision WINS, so spans and logs never disagree about
     * whether a request was recorded.
     */
    traceSampled?: boolean;
    /** Kill switch — false disables all telemetry emission, errors included. */
    enabled: boolean;
    /** Minimum level to emit. */
    minimumLevel: CanonicalLevel;
    /** Session-scoped browser log sampling ratio. */
    logSamplingRatio: number;
}

/**
 * The single decision point for "does this browser log line get emitted?".
 *
 * Order matters and is part of the parity contract:
 *   1. kill switch  — off means off, no exceptions
 *   2. minimum level — below the floor is not emitted
 *   3. WARN/ERROR/FATAL — always 100%, sampled-out session or not (ADR-010:
 *      "sampling errors is malpractice")
 *   4. trace decision, if a trace exists — inherited, never re-rolled
 *   5. otherwise the session decision — one roll for the whole session
 *
 * Server-side logs are NOT sampled (ADR-010, unchanged by ADR-097): callers on
 * the server simply do not route through here.
 */
export function shouldEmitLog(input: LogSamplingInput): boolean {
    if (!input.enabled) return false;
    const level = normalizeLevel(input.level);
    if (!meetsMinimumLevel(level, input.minimumLevel)) return false;
    if (LEVEL_RANK[level] >= LEVEL_RANK.WARN) return true;
    if (input.traceSampled !== undefined) return input.traceSampled;
    return sampleDecision(input.sessionId, input.logSamplingRatio);
}

/**
 * "Sampled-out sessions still send a periodic count so volume is observable
 * even when content is not" (ADR-097). Dropping data silently is the thing we
 * are trying to avoid, at every level.
 *
 * This is just the tally; W3 owns flushing it on the browser log transport.
 */
export interface DropCounter {
    /** Record a line that the sampler suppressed. */
    record(level: CanonicalLevel): void;
    /** Return the counts so far and reset. Empty object when nothing dropped. */
    drain(): Partial<Record<CanonicalLevel, number>>;
}

export function createDropCounter(): DropCounter {
    let counts: Partial<Record<CanonicalLevel, number>> = {};
    return {
        record(level) {
            counts[level] = (counts[level] ?? 0) + 1;
        },
        drain() {
            const out = counts;
            counts = {};
            return out;
        },
    };
}
