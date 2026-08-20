// ADR-097 — session-scoped sampling. Go port of packages/core/src/sampling.ts.
//
// THE CORE RULE: the sampling decision is made ONCE per session (or per trace,
// where one exists) and applies to EVERY log line under it. The invariant this
// buys: any trace you can open has 100% of its log lines. Never a partial view.
//
// Every vector this file must reproduce lives in parity/sampling-corpus.json
// and is asserted by parity_corpus_test.go — the same file the TS, Rust, Python
// and .NET lanes load. See parity/README.md for the hash derivation and the
// porting traps.

package observability

import (
	"math"
	"strings"
)

const (
	fnvOffsetBasis32 uint32 = 0x811c9dc5
	fnvPrime32       uint32 = 0x01000193
	// twoPow32 as an exact float64 — dividing a uint32 by this is a pure
	// exponent adjustment, so every language gets the identical double.
	twoPow32 float64 = 4294967296.0
)

// FNV1a32 computes FNV-1a 32-bit over the UTF-8 bytes of s.
//
// XOR-then-multiply (this is FNV-*1a*, not FNV-1). Go's uint32 arithmetic wraps
// natively; ranging over a string with an index yields bytes, not runes, which
// is what the byte-at-a-time fold requires.
func FNV1a32(s string) uint32 {
	h := fnvOffsetBasis32
	for i := 0; i < len(s); i++ {
		h ^= uint32(s[i])
		h *= fnvPrime32
	}
	return h
}

// SampleDecision is the one sampling primitive. Deterministic and stable for
// the lifetime of an id.
//
//   - non-finite ratio -> IN (fail open — telemetry going dark on a config
//     hiccup is the failure ADR-097 forbids; x < NaN is false everywhere, so the
//     naive path would sample *everything* out)
//   - ratio <= 0.0 -> OUT, ratio >= 1.0 -> IN, both taken before any float math
//     so 1.0 can never drop and 0.0 can never keep
//   - otherwise (hash / 2^32) < ratio, strict less-than
func SampleDecision(id string, ratio float64) bool {
	if math.IsNaN(ratio) || math.IsInf(ratio, 0) {
		return true
	}
	if ratio <= 0.0 {
		return false
	}
	if ratio >= 1.0 {
		return true
	}
	// float64(uint32) is exact — the uint32 must NOT be routed through int32,
	// which would make hashes above 2^31 negative and flip their decision.
	return float64(FNV1a32(id))/twoPow32 < ratio
}

// CanonicalLevel is an ADR-096 canonical log level, uppercase.
//
// Distinct from Level, which is the lowercase *event wire* severity on the
// ingest envelope. This is the log-line severity the ClickHouse queries filter
// on, and `level IN ('ERROR','FATAL')` is CASE-SENSITIVE — emitting "error"
// silently makes every error a non-error.
type CanonicalLevel string

const (
	CanonicalTrace CanonicalLevel = "TRACE"
	CanonicalDebug CanonicalLevel = "DEBUG"
	CanonicalInfo  CanonicalLevel = "INFO"
	CanonicalWarn  CanonicalLevel = "WARN"
	CanonicalError CanonicalLevel = "ERROR"
	CanonicalFatal CanonicalLevel = "FATAL"
)

// levelRank is the ordering used by the minimum-level filter. Matches OTel
// severity numbers.
var levelRank = map[CanonicalLevel]int{
	CanonicalTrace: 1,
	CanonicalDebug: 5,
	CanonicalInfo:  9,
	CanonicalWarn:  13,
	CanonicalError: 17,
	CanonicalFatal: 21,
}

// levelAliases are the spellings every SDK must accept. Part of the parity
// contract.
var levelAliases = map[string]CanonicalLevel{
	"trace":       CanonicalTrace,
	"verbose":     CanonicalTrace,
	"debug":       CanonicalDebug,
	"info":        CanonicalInfo,
	"information": CanonicalInfo,
	"log":         CanonicalInfo,
	"notice":      CanonicalInfo,
	"warn":        CanonicalWarn,
	"warning":     CanonicalWarn,
	"error":       CanonicalError,
	"err":         CanonicalError,
	"fatal":       CanonicalFatal,
	"critical":    CanonicalFatal,
	"crit":        CanonicalFatal,
	"emergency":   CanonicalFatal,
	"panic":       CanonicalFatal,
}

// ParseLevel is a strict parse: a known level spelling, or ok=false.
func ParseLevel(level string) (CanonicalLevel, bool) {
	l, ok := levelAliases[strings.ToLower(strings.TrimSpace(level))]
	return l, ok
}

// NormalizeLevel normalizes any level spelling to the canonical form.
//
// Unknown spellings become INFO — fail-safe: an unrecognised level must never
// cause a drop, and must never be promoted into ERROR (which would corrupt the
// error rate).
func NormalizeLevel(level string) CanonicalLevel {
	if l, ok := ParseLevel(level); ok {
		return l
	}
	return CanonicalInfo
}

// MeetsMinimumLevel reports whether level is at or above minimum.
func MeetsMinimumLevel(level, minimum CanonicalLevel) bool {
	return levelRank[level] >= levelRank[minimum]
}

// LogSamplingInput is the input to ShouldEmitLog.
type LogSamplingInput struct {
	// Level as emitted by the caller; normalized internally.
	Level string
	// SessionID is the stable per-page session id, used when no trace exists.
	SessionID string
	// TraceSampled is the trace's own sampling decision, when a trace context
	// exists (nil when it does not). Where a trace exists its decision WINS, so
	// spans and logs never disagree.
	TraceSampled *bool
	// Enabled is the kill switch — false disables all emission, errors included.
	Enabled bool
	// MinimumLevel is the floor to emit at.
	MinimumLevel CanonicalLevel
	// LogSamplingRatio is the session-scoped browser log sampling ratio.
	LogSamplingRatio float64
}

// ShouldEmitLog is the single decision point for "does this log line get
// emitted?".
//
// Order matters and is part of the parity contract:
//  1. kill switch — off means off, no exceptions
//  2. minimum level — below the floor is not emitted
//  3. WARN/ERROR/FATAL — always 100% (ADR-010: "sampling errors is malpractice")
//  4. trace decision, if a trace exists — inherited, never re-rolled
//  5. otherwise the session decision — one roll for the whole session
func ShouldEmitLog(in LogSamplingInput) bool {
	if !in.Enabled {
		return false
	}
	level := NormalizeLevel(in.Level)
	if !MeetsMinimumLevel(level, in.MinimumLevel) {
		return false
	}
	if levelRank[level] >= levelRank[CanonicalWarn] {
		return true
	}
	if in.TraceSampled != nil {
		return *in.TraceSampled
	}
	return SampleDecision(in.SessionID, in.LogSamplingRatio)
}
