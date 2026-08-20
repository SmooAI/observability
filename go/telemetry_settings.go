// ADR-097 W1 — config-served telemetry settings. Go port of
// packages/core/src/telemetry-settings.ts, pinned by
// parity/sampling-corpus.json.
//
// These are @smooai/config public-tier, org-scoped keys. Public tier is
// mandatory: a browser can never be served secret tier (ADR-075), and the whole
// point is that changing a key changes every client's behaviour on its next
// config read. No secret may ever enter this key set.
//
// FAIL-SAFE IS THE POINT. Unreachable provider, malformed payload,
// out-of-range value -> the compiled-in ADR-010 defaults. Never "sample
// everything out": a telemetry system that goes silent when its config server
// hiccups is worse than useless. A caller whose config read *failed* passes nil
// and gets exactly those defaults — which is why this function has no error
// return.

package observability

import (
	"encoding/json"
	"math"
	"strconv"
	"strings"
)

// The @smooai/config public-tier key names this SDK reads.
const (
	// KeyObservabilityEnabled is the boolean kill switch. false disables ALL
	// telemetry emission.
	KeyObservabilityEnabled = "observabilityEnabled"
	// KeyBrowserLogSamplingRatio is a number 0.0–1.0, the session-scoped
	// browser log sampling ratio.
	KeyBrowserLogSamplingRatio = "observabilityBrowserLogSamplingRatio"
	// KeyMinimumLogLevel is the minimum log level to emit.
	KeyMinimumLogLevel = "observabilityMinimumLogLevel"
	// KeyTraceSamplingRatio is a number 0.0–1.0, the head-based trace sampling
	// ratio.
	KeyTraceSamplingRatio = "observabilityTraceSamplingRatio"
)

// TelemetrySettings are the resolved telemetry settings.
type TelemetrySettings struct {
	// Enabled is the kill switch. When false nothing is emitted, errors
	// included.
	Enabled bool
	// BrowserLogSamplingRatio is applied ONCE per session (or inherited from
	// the trace where one exists) — never per line.
	BrowserLogSamplingRatio float64
	// MinimumLogLevel is the floor to emit at.
	MinimumLogLevel CanonicalLevel
	// TraceSamplingRatio is head-based. ADR-010 default: TraceIdRatioBased(0.1).
	TraceSamplingRatio float64
}

// DefaultTelemetrySettings returns the compiled-in ADR-010 defaults. Every
// failure path lands here.
func DefaultTelemetrySettings() TelemetrySettings {
	return TelemetrySettings{
		Enabled:                 true,
		BrowserLogSamplingRatio: 1.0,
		MinimumLogLevel:         CanonicalInfo,
		TraceSamplingRatio:      0.1,
	}
}

// coerceRatio accepts a finite number, or a decimal numeric string (public
// config often round-trips values as strings), clamped into [0, 1] — an
// operator who writes 1.5 means "all". Anything else (missing, NaN, Infinity,
// boolean, object, unparseable string) falls back to the default, never 0.
//
// The asymmetry is deliberate: a *malformed* value falls back, a *valid but
// out-of-range* value is clamped. -1 clamps to 0 (telemetry off) because that
// is an explicit operator value, and 0 is settable anyway.
func coerceRatio(raw any, fallback float64) float64 {
	var n float64
	switch v := raw.(type) {
	case float64:
		n = v
	case float32:
		n = float64(v)
	case int:
		n = float64(v)
	case int64:
		n = float64(v)
	case json.Number:
		parsed, err := v.Float64()
		if err != nil {
			return fallback
		}
		n = parsed
	case string:
		s := strings.TrimSpace(v)
		if s == "" {
			return fallback
		}
		parsed, err := strconv.ParseFloat(s, 64)
		if err != nil {
			return fallback
		}
		n = parsed
	default:
		return fallback
	}
	if math.IsNaN(n) || math.IsInf(n, 0) {
		return fallback
	}
	return math.Min(1, math.Max(0, n))
}

func coerceBool(raw any, fallback bool) bool {
	switch v := raw.(type) {
	case bool:
		return v
	case string:
		switch strings.ToLower(strings.TrimSpace(v)) {
		case "true":
			return true
		case "false":
			return false
		}
	}
	return fallback
}

// coerceLevel uses ParseLevel (not NormalizeLevel) on purpose: normalize maps
// unknown spellings to INFO, which is right for an incoming log line but wrong
// here — a typo'd config value must fall back to the default, not silently
// reset the floor.
func coerceLevel(raw any, fallback CanonicalLevel) CanonicalLevel {
	if s, ok := raw.(string); ok {
		if l, ok := ParseLevel(s); ok {
			return l
		}
	}
	return fallback
}

// ResolveTelemetrySettings turns a raw config payload (typically the result of
// json.Unmarshal into an any) into settings. Total function — never fails,
// always returns a usable value. Unknown/extra keys are ignored.
func ResolveTelemetrySettings(raw any) TelemetrySettings {
	d := DefaultTelemetrySettings()
	bag, ok := raw.(map[string]any)
	if !ok {
		return d
	}
	return TelemetrySettings{
		Enabled:                 coerceBool(bag[KeyObservabilityEnabled], d.Enabled),
		BrowserLogSamplingRatio: coerceRatio(bag[KeyBrowserLogSamplingRatio], d.BrowserLogSamplingRatio),
		MinimumLogLevel:         coerceLevel(bag[KeyMinimumLogLevel], d.MinimumLogLevel),
		TraceSamplingRatio:      coerceRatio(bag[KeyTraceSamplingRatio], d.TraceSamplingRatio),
	}
}
