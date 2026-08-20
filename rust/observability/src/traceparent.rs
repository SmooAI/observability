//! W3C `traceparent` parse / format. Rust port of
//! `packages/core/src/traceparent.ts`, pinned by `parity/sampling-corpus.json`.
//!
//! ```text
//! 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
//! ^  ^                                ^                ^
//! |  trace-id (16 bytes, 32 lowercase hex)             |
//! |                                   span-id (8 bytes, 16 lowercase hex)
//! version (1 byte, 2 hex)                              trace-flags (1 byte, 2 hex)
//! ```
//!
//! Parsing is STRICT — exactly four dash-separated fields, version exactly
//! `00`. Rejected: wrong field count, wrong version (including the forbidden
//! `ff`), non-hex or wrong-length fields, uppercase hex, an all-zero trace id,
//! an all-zero span id. The all-zero ids are the classic "propagated a
//! placeholder" bug: accepting them produces traces that all collide on `000…0`.

const VERSION: &str = "00";

/// A parsed `traceparent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    /// 32 lowercase hex chars.
    pub trace_id: String,
    /// 16 lowercase hex chars.
    pub span_id: String,
    /// trace-flags byte.
    pub flags: u8,
    /// bit 0 of `flags` — the upstream sampling decision, which logs inherit.
    pub sampled: bool,
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parse a `traceparent` header. Returns `None` for anything invalid.
pub fn parse_traceparent(header: &str) -> Option<TraceContext> {
    let parts: Vec<&str> = header.split('-').collect();
    if parts.len() != 4 {
        return None;
    }
    let (version, trace_id, span_id, flags_hex) = (parts[0], parts[1], parts[2], parts[3]);
    if version != VERSION {
        return None;
    }
    if !is_lower_hex(trace_id, 32) || trace_id.bytes().all(|b| b == b'0') {
        return None;
    }
    if !is_lower_hex(span_id, 16) || span_id.bytes().all(|b| b == b'0') {
        return None;
    }
    if !is_lower_hex(flags_hex, 2) {
        return None;
    }
    let flags = u8::from_str_radix(flags_hex, 16).ok()?;
    Some(TraceContext {
        trace_id: trace_id.to_string(),
        span_id: span_id.to_string(),
        flags,
        sampled: flags & 0x01 == 0x01,
    })
}

/// Format a `traceparent` header.
///
/// `flags` is an `i64` rather than a `u8` so an out-of-byte-range value from a
/// caller (or from the parity corpus, which pins 256) is *rejected* rather than
/// silently truncated. Pass `None` to derive the flags byte from `sampled`.
///
/// Returns `None` rather than emitting a header a spec-compliant peer would
/// reject — a malformed traceparent breaks correlation downstream just as
/// thoroughly as a missing one, but silently.
pub fn format_traceparent(
    trace_id: &str,
    span_id: &str,
    flags: Option<i64>,
    sampled: Option<bool>,
) -> Option<String> {
    let flags = flags.unwrap_or(if sampled.unwrap_or(false) { 1 } else { 0 });
    if !(0..=255).contains(&flags) {
        return None;
    }
    let header = format!("{VERSION}-{trace_id}-{span_id}-{flags:02x}");
    // Round-trip through the parser so format can never emit what parse rejects.
    parse_traceparent(&header).map(|_| header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let h = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let ctx = parse_traceparent(h).expect("valid");
        assert!(ctx.sampled);
        assert_eq!(
            format_traceparent(
                &ctx.trace_id,
                &ctx.span_id,
                Some(i64::from(ctx.flags)),
                None
            )
            .as_deref(),
            Some(h)
        );
    }

    #[test]
    fn format_never_emits_what_parse_rejects() {
        assert_eq!(
            format_traceparent("0".repeat(32).as_str(), "00f067aa0ba902b7", Some(1), None),
            None
        );
        assert_eq!(
            format_traceparent(
                "4bf92f3577b34da6a3ce929d0e0e4736",
                "00f067aa0ba902b7",
                Some(256),
                None
            ),
            None
        );
    }
}
