// W3C traceparent parse / format. Go port of packages/core/src/traceparent.ts,
// pinned by parity/sampling-corpus.json.
//
//	00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
//	^  ^                                ^                ^
//	|  trace-id (16 bytes, 32 lowercase hex)             |
//	|                                   span-id (8 bytes, 16 lowercase hex)
//	version (1 byte, 2 hex)                              trace-flags (1 byte, 2 hex)
//
// Parsing is STRICT — exactly four dash-separated fields, version exactly "00".
// Rejected: wrong field count, wrong version (including the forbidden "ff"),
// non-hex or wrong-length fields, uppercase hex, an all-zero trace id, an
// all-zero span id. The all-zero ids are the classic "propagated a placeholder"
// bug: accepting them produces traces that all collide on 000…0.

package observability

import (
	"fmt"
	"strconv"
	"strings"
)

const traceparentVersion = "00"

// TraceContext is a parsed traceparent.
type TraceContext struct {
	// TraceID is 32 lowercase hex chars.
	TraceID string
	// SpanID is 16 lowercase hex chars.
	SpanID string
	// Flags is the trace-flags byte.
	Flags uint8
	// Sampled is bit 0 of Flags — the upstream sampling decision logs inherit.
	Sampled bool
}

// isLowerHex reports whether s is exactly n lowercase hex digits. strconv's hex
// parsers accept uppercase, so the case check has to be explicit.
func isLowerHex(s string, n int) bool {
	if len(s) != n {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if (c < '0' || c > '9') && (c < 'a' || c > 'f') {
			return false
		}
	}
	return true
}

func isAllZero(s string) bool {
	return strings.Count(s, "0") == len(s)
}

// ParseTraceparent parses a traceparent header. ok is false for anything
// invalid.
func ParseTraceparent(header string) (TraceContext, bool) {
	parts := strings.Split(header, "-")
	if len(parts) != 4 {
		return TraceContext{}, false
	}
	version, traceID, spanID, flagsHex := parts[0], parts[1], parts[2], parts[3]
	if version != traceparentVersion {
		return TraceContext{}, false
	}
	if !isLowerHex(traceID, 32) || isAllZero(traceID) {
		return TraceContext{}, false
	}
	if !isLowerHex(spanID, 16) || isAllZero(spanID) {
		return TraceContext{}, false
	}
	if !isLowerHex(flagsHex, 2) {
		return TraceContext{}, false
	}
	flags, err := strconv.ParseUint(flagsHex, 16, 8)
	if err != nil {
		return TraceContext{}, false
	}
	return TraceContext{TraceID: traceID, SpanID: spanID, Flags: uint8(flags), Sampled: flags&0x01 == 0x01}, true
}

// FormatTraceparent formats a traceparent header.
//
// flags is an int (not a uint8) so an out-of-byte-range value from a caller is
// rejected rather than silently truncated. Pass nil to derive the flags byte
// from sampled.
//
// ok is false rather than emitting a header a spec-compliant peer would reject
// — a malformed traceparent breaks correlation downstream just as thoroughly as
// a missing one, but silently.
func FormatTraceparent(traceID, spanID string, flags *int, sampled *bool) (string, bool) {
	resolved := 0
	switch {
	case flags != nil:
		resolved = *flags
	case sampled != nil && *sampled:
		resolved = 1
	}
	if resolved < 0 || resolved > 255 {
		return "", false
	}
	header := fmt.Sprintf("%s-%s-%s-%02x", traceparentVersion, traceID, spanID, resolved)
	// Round-trip through the parser so format can never emit what parse rejects.
	if _, ok := ParseTraceparent(header); !ok {
		return "", false
	}
	return header, true
}
