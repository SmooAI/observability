/**
 * W3C `traceparent` parse / format.
 *
 * ADR-097 §4: ADR-007/009 documented traceparent propagation as working; it
 * appears in zero source files across five SDKs. This is the first real
 * implementation, and the corpus (`parity/sampling-corpus.json`) pins it so the
 * ports cannot quietly diverge again.
 *
 * Wire format (https://www.w3.org/TR/trace-context/):
 *
 *     00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
 *     ^  ^                                ^                ^
 *     |  trace-id (16 bytes, 32 lowercase hex)             |
 *     |                                   span-id (8 bytes, 16 lowercase hex)
 *     version (1 byte, 2 hex)                              trace-flags (1 byte, 2 hex)
 *
 * Parsing is STRICT — exactly four dash-separated fields, version exactly `00`.
 * The spec permits future versions to append fields, but we have no forward
 * version to be compatible with and a strict parser is trivially portable; a
 * porter must reject anything not `00-…` with exactly four fields.
 *
 * Rejected (returns null): wrong field count, wrong version (including the
 * forbidden `ff`), non-hex or wrong-length fields, uppercase hex, an all-zero
 * trace id, an all-zero span id. The all-zero ids are invalid per spec and are
 * the classic "propagated a placeholder" bug — accepting them produces traces
 * that all collide on id `000…0`.
 *
 * `sampled` is bit 0 of trace-flags.
 */

export interface TraceContext {
    /** 32 lowercase hex chars. */
    traceId: string;
    /** 16 lowercase hex chars. */
    spanId: string;
    /** trace-flags byte, 0-255. */
    flags: number;
    /** bit 0 of `flags` — the upstream sampling decision, which logs inherit. */
    sampled: boolean;
}

const VERSION = '00';
const HEX32 = /^[0-9a-f]{32}$/;
const HEX16 = /^[0-9a-f]{16}$/;
const HEX2 = /^[0-9a-f]{2}$/;
const ZERO_TRACE_ID = '0'.repeat(32);
const ZERO_SPAN_ID = '0'.repeat(16);

/** Parse a `traceparent` header. Returns null for anything invalid. */
export function parseTraceparent(header: string): TraceContext | null {
    const parts = header.split('-');
    if (parts.length !== 4) return null;
    const [version, traceId, spanId, flagsHex] = parts as [string, string, string, string];
    if (version !== VERSION) return null;
    if (!HEX32.test(traceId) || traceId === ZERO_TRACE_ID) return null;
    if (!HEX16.test(spanId) || spanId === ZERO_SPAN_ID) return null;
    if (!HEX2.test(flagsHex)) return null;
    const flags = parseInt(flagsHex, 16);
    return { traceId, spanId, flags, sampled: (flags & 0x01) === 0x01 };
}

/**
 * Format a `traceparent` header. Returns null rather than emitting a header
 * that a spec-compliant peer would reject — a malformed traceparent breaks
 * correlation downstream just as thoroughly as a missing one, but silently.
 */
export function formatTraceparent(ctx: Pick<TraceContext, 'traceId' | 'spanId'> & { flags?: number; sampled?: boolean }): string | null {
    const flags = ctx.flags ?? (ctx.sampled ? 1 : 0);
    if (!Number.isInteger(flags) || flags < 0 || flags > 255) return null;
    const header = `${VERSION}-${ctx.traceId}-${ctx.spanId}-${flags.toString(16).padStart(2, '0')}`;
    // Round-trip through the parser so format can never emit what parse rejects.
    return parseTraceparent(header) ? header : null;
}
