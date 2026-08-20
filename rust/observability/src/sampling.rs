//! ADR-097 — session-scoped sampling. Rust port of `packages/core/src/sampling.ts`.
//!
//! THE CORE RULE: the sampling decision is made ONCE per session (or per trace,
//! where one exists) and applies to EVERY log line under it. The invariant this
//! buys: **any trace you can open has 100% of its log lines**. Never a partial
//! view.
//!
//! Every vector this module must reproduce lives in
//! `parity/sampling-corpus.json` and is asserted by
//! `tests/parity_corpus.rs` — the same file the TS, Python, Go and .NET lanes
//! load. See `parity/README.md` for the hash derivation and the porting traps.

/// FNV offset basis, 32-bit.
const FNV_OFFSET_BASIS_32: u32 = 0x811c_9dc5;
/// FNV prime, 32-bit.
const FNV_PRIME_32: u32 = 0x0100_0193;
/// 2^32 as an exact binary64 — dividing a u32 by this is a pure exponent
/// adjustment, so every language gets the identical double.
const TWO_POW_32: f64 = 4_294_967_296.0;

/// FNV-1a 32-bit over the UTF-8 bytes of `input`.
///
/// XOR-then-multiply (this is FNV-*1a*, not FNV-1) with wrapping 32-bit
/// arithmetic. Byte-at-a-time, so there is no endianness to get wrong.
pub fn fnv1a32(input: &str) -> u32 {
    let mut h = FNV_OFFSET_BASIS_32;
    for b in input.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(FNV_PRIME_32);
    }
    h
}

/// The one sampling primitive. Deterministic and stable for the lifetime of an
/// id.
///
/// * non-finite `ratio` → IN (fail open — telemetry going dark on a config
///   hiccup is the failure ADR-097 forbids; `x < NaN` is false everywhere, so
///   the naive path would sample *everything* out)
/// * `ratio <= 0.0` → OUT, `ratio >= 1.0` → IN, both taken before any float
///   math so 1.0 can never drop and 0.0 can never keep
/// * otherwise `(hash / 2^32) < ratio`, strict less-than
pub fn sample_decision(id: &str, ratio: f64) -> bool {
    if !ratio.is_finite() {
        return true;
    }
    if ratio <= 0.0 {
        return false;
    }
    if ratio >= 1.0 {
        return true;
    }
    f64::from(fnv1a32(id)) / TWO_POW_32 < ratio
}

/// Canonical log levels, uppercase (ADR-096).
///
/// Distinct from [`crate::Level`], which is the lowercase *event wire* severity
/// on the ingest envelope. This is the log-line severity the ClickHouse queries
/// filter on, and `level IN ('ERROR','FATAL')` is CASE-SENSITIVE — emitting
/// `"error"` silently makes every error a non-error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl CanonicalLevel {
    /// The canonical uppercase spelling written to the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            CanonicalLevel::Trace => "TRACE",
            CanonicalLevel::Debug => "DEBUG",
            CanonicalLevel::Info => "INFO",
            CanonicalLevel::Warn => "WARN",
            CanonicalLevel::Error => "ERROR",
            CanonicalLevel::Fatal => "FATAL",
        }
    }

    /// Ordering used by the minimum-level filter. Matches OTel severity numbers.
    pub fn rank(self) -> u8 {
        match self {
            CanonicalLevel::Trace => 1,
            CanonicalLevel::Debug => 5,
            CanonicalLevel::Info => 9,
            CanonicalLevel::Warn => 13,
            CanonicalLevel::Error => 17,
            CanonicalLevel::Fatal => 21,
        }
    }
}

impl std::fmt::Display for CanonicalLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Strict parse: a known level spelling (case-insensitive, surrounding
/// whitespace trimmed), or `None`.
///
/// The alias set is part of the parity contract — every SDK accepts exactly
/// these spellings.
pub fn parse_level(level: &str) -> Option<CanonicalLevel> {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" | "verbose" => Some(CanonicalLevel::Trace),
        "debug" => Some(CanonicalLevel::Debug),
        "info" | "information" | "log" | "notice" => Some(CanonicalLevel::Info),
        "warn" | "warning" => Some(CanonicalLevel::Warn),
        "error" | "err" => Some(CanonicalLevel::Error),
        "fatal" | "critical" | "crit" | "emergency" | "panic" => Some(CanonicalLevel::Fatal),
        _ => None,
    }
}

/// Normalize any level spelling to the canonical form.
///
/// Unknown spellings become `INFO` — fail-safe: an unrecognised level must
/// never cause a drop, and must never be promoted into ERROR (which would
/// corrupt the error rate).
pub fn normalize_level(level: &str) -> CanonicalLevel {
    parse_level(level).unwrap_or(CanonicalLevel::Info)
}

/// True when `level` is at or above `minimum`.
pub fn meets_minimum_level(level: CanonicalLevel, minimum: CanonicalLevel) -> bool {
    level.rank() >= minimum.rank()
}

/// Input to [`should_emit_log`].
#[derive(Debug, Clone)]
pub struct LogSamplingInput<'a> {
    /// Level as emitted by the caller; normalized internally.
    pub level: &'a str,
    /// Stable per-page session id. Used when no trace context exists.
    pub session_id: &'a str,
    /// The trace's own sampling decision, when a trace context exists. Where a
    /// trace exists its decision WINS, so spans and logs never disagree.
    pub trace_sampled: Option<bool>,
    /// Kill switch — `false` disables all telemetry emission, errors included.
    pub enabled: bool,
    /// Minimum level to emit.
    pub minimum_level: CanonicalLevel,
    /// Session-scoped browser log sampling ratio.
    pub log_sampling_ratio: f64,
}

/// The single decision point for "does this log line get emitted?".
///
/// Order matters and is part of the parity contract:
///   1. kill switch — off means off, no exceptions
///   2. minimum level — below the floor is not emitted
///   3. WARN/ERROR/FATAL — always 100% (ADR-010: "sampling errors is malpractice")
///   4. trace decision, if a trace exists — inherited, never re-rolled
///   5. otherwise the session decision — one roll for the whole session
pub fn should_emit_log(input: &LogSamplingInput<'_>) -> bool {
    if !input.enabled {
        return false;
    }
    let level = normalize_level(input.level);
    if !meets_minimum_level(level, input.minimum_level) {
        return false;
    }
    if level.rank() >= CanonicalLevel::Warn.rank() {
        return true;
    }
    if let Some(sampled) = input.trace_sampled {
        return sampled;
    }
    sample_decision(input.session_id, input.log_sampling_ratio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_fnv_vectors() {
        // The published FNV-1a-32 vectors — an independent check that this is
        // FNV-1a and not FNV-1 (which would give different values entirely).
        assert_eq!(fnv1a32(""), 0x811c_9dc5);
        assert_eq!(fnv1a32("a"), 0xe40c_292c);
        assert_eq!(fnv1a32("foobar"), 0xbf9c_f968);
    }

    #[test]
    fn ratio_bounds_are_exact() {
        assert!(!sample_decision("anything", 0.0));
        assert!(sample_decision("anything", 1.0));
        assert!(sample_decision("anything", f64::NAN));
    }

    #[test]
    fn unknown_level_is_info_not_error() {
        assert_eq!(normalize_level("bogus"), CanonicalLevel::Info);
        assert_eq!(parse_level("bogus"), None);
    }
}
