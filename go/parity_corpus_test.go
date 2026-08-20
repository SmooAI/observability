// ADR-097 §4 — the Go lane of the parity corpus.
//
// Every SDK (TS, Rust, Python, Go, .NET) asserts against the same
// parity/sampling-corpus.json in its own CI. A language that cannot reproduce a
// vector fails its build. Documentation claiming parity is not evidence of
// parity.

package observability

import (
	"encoding/json"
	"math"
	"os"
	"testing"
)

// The corpus lives at the repo root, one level above this module.
const corpusPath = "../parity/sampling-corpus.json"

func loadCorpus(t *testing.T) map[string]json.RawMessage {
	t.Helper()
	raw, err := os.ReadFile(corpusPath)
	if err != nil {
		t.Fatalf("parity corpus unreadable at %s: %v", corpusPath, err)
	}
	var c map[string]json.RawMessage
	if err := json.Unmarshal(raw, &c); err != nil {
		t.Fatalf("parity corpus is not valid JSON: %v", err)
	}
	return c
}

// section decodes one named array of the corpus into out.
func section(t *testing.T, name string, out any) {
	t.Helper()
	c := loadCorpus(t)
	raw, ok := c[name]
	if !ok {
		t.Fatalf("corpus section %q missing", name)
	}
	if err := json.Unmarshal(raw, out); err != nil {
		t.Fatalf("corpus section %q does not decode: %v", name, err)
	}
}

func TestCorpusIsTheExpectedVersionAndIsNotEmpty(t *testing.T) {
	var version int
	c := loadCorpus(t)
	if err := json.Unmarshal(c["version"], &version); err != nil || version != 1 {
		t.Fatalf("corpus schema version is %v (%v) — re-read parity/README.md before bumping", version, err)
	}
	var vectors []struct{}
	section(t, "sampleDecision", &vectors)
	if len(vectors) <= 50 {
		t.Fatalf("sampleDecision has only %d vectors — the corpus looks truncated", len(vectors))
	}
}

func TestSampleDecisionVectors(t *testing.T) {
	var vectors []struct {
		ID       string  `json:"id"`
		Ratio    float64 `json:"ratio"`
		Hash     uint32  `json:"hash"`
		Expected bool    `json:"expected"`
	}
	section(t, "sampleDecision", &vectors)
	for _, v := range vectors {
		if got := FNV1a32(v.ID); got != v.Hash {
			t.Errorf("FNV1a32(%q) = %d, want %d", v.ID, got, v.Hash)
		}
		if got := SampleDecision(v.ID, v.Ratio); got != v.Expected {
			t.Errorf("SampleDecision(%q, %v) = %v, want %v", v.ID, v.Ratio, got, v.Expected)
		}
	}
}

func TestSampleDecisionNearThresholdVectors(t *testing.T) {
	var vectors []struct {
		ID       string  `json:"id"`
		Ratio    float64 `json:"ratio"`
		Hash     uint32  `json:"hash"`
		Position float64 `json:"position"`
		Expected bool    `json:"expected"`
	}
	section(t, "sampleDecisionNearThreshold", &vectors)
	for _, v := range vectors {
		h := FNV1a32(v.ID)
		if h != v.Hash {
			t.Errorf("FNV1a32(%q) = %d, want %d", v.ID, h, v.Hash)
		}
		// The division is exact in float64, so this is an equality check, not
		// an epsilon one — drift here means a language got the uint32
		// reinterpretation or the divisor wrong.
		if pos := float64(h) / 4294967296.0; pos != v.Position {
			t.Errorf("position(%q) = %v, want %v", v.ID, pos, v.Position)
		}
		if got := SampleDecision(v.ID, v.Ratio); got != v.Expected {
			t.Errorf("SampleDecision(%q, %v) = %v, want %v", v.ID, v.Ratio, got, v.Expected)
		}
	}
}

func TestNonFiniteRatioFailsOpen(t *testing.T) {
	var vectors []struct {
		ID       string `json:"id"`
		Ratio    string `json:"ratio"`
		Expected bool   `json:"expected"`
	}
	section(t, "sampleDecisionNonFiniteRatio", &vectors)
	nonFinite := map[string]float64{"NaN": math.NaN(), "Infinity": math.Inf(1), "-Infinity": math.Inf(-1)}
	for _, v := range vectors {
		ratio, ok := nonFinite[v.Ratio]
		if !ok {
			t.Fatalf("corpus names an unknown non-finite ratio: %s", v.Ratio)
		}
		if got := SampleDecision(v.ID, ratio); got != v.Expected {
			t.Errorf("SampleDecision(%q, %s) = %v, want %v", v.ID, v.Ratio, got, v.Expected)
		}
	}
}

func TestLevelNormalizationVectors(t *testing.T) {
	var vectors []struct {
		Input    string `json:"input"`
		Expected string `json:"expected"`
	}
	section(t, "levelNormalization", &vectors)
	for _, v := range vectors {
		if got := NormalizeLevel(v.Input); string(got) != v.Expected {
			t.Errorf("NormalizeLevel(%q) = %q, want %q", v.Input, got, v.Expected)
		}
	}
}

func TestTraceparentParseVectors(t *testing.T) {
	var vectors []struct {
		Input    string `json:"input"`
		Expected *struct {
			TraceID string `json:"traceId"`
			SpanID  string `json:"spanId"`
			Flags   uint8  `json:"flags"`
			Sampled bool   `json:"sampled"`
		} `json:"expected"`
	}
	section(t, "traceparentParse", &vectors)
	for _, v := range vectors {
		got, ok := ParseTraceparent(v.Input)
		if v.Expected == nil {
			if ok {
				t.Errorf("ParseTraceparent(%q) accepted %+v, want rejection", v.Input, got)
			}
			continue
		}
		if !ok {
			t.Errorf("ParseTraceparent(%q) rejected, want %+v", v.Input, *v.Expected)
			continue
		}
		if got.TraceID != v.Expected.TraceID || got.SpanID != v.Expected.SpanID || got.Flags != v.Expected.Flags || got.Sampled != v.Expected.Sampled {
			t.Errorf("ParseTraceparent(%q) = %+v, want %+v", v.Input, got, *v.Expected)
		}
	}
}

func TestTraceparentFormatVectors(t *testing.T) {
	var vectors []struct {
		Input struct {
			TraceID string `json:"traceId"`
			SpanID  string `json:"spanId"`
			Flags   *int   `json:"flags"`
			Sampled *bool  `json:"sampled"`
		} `json:"input"`
		Expected *string `json:"expected"`
	}
	section(t, "traceparentFormat", &vectors)
	for _, v := range vectors {
		got, ok := FormatTraceparent(v.Input.TraceID, v.Input.SpanID, v.Input.Flags, v.Input.Sampled)
		if v.Expected == nil {
			if ok {
				t.Errorf("FormatTraceparent(%+v) = %q, want rejection", v.Input, got)
			}
			continue
		}
		if !ok || got != *v.Expected {
			t.Errorf("FormatTraceparent(%+v) = %q (ok=%v), want %q", v.Input, got, ok, *v.Expected)
		}
	}
}

func TestSettingsResolutionVectors(t *testing.T) {
	var vectors []struct {
		Input    any `json:"input"`
		Expected struct {
			Enabled                 bool    `json:"enabled"`
			BrowserLogSamplingRatio float64 `json:"browserLogSamplingRatio"`
			MinimumLogLevel         string  `json:"minimumLogLevel"`
			TraceSamplingRatio      float64 `json:"traceSamplingRatio"`
		} `json:"expected"`
	}
	section(t, "settingsResolution", &vectors)
	for _, v := range vectors {
		got := ResolveTelemetrySettings(v.Input)
		if got.Enabled != v.Expected.Enabled ||
			got.BrowserLogSamplingRatio != v.Expected.BrowserLogSamplingRatio ||
			string(got.MinimumLogLevel) != v.Expected.MinimumLogLevel ||
			got.TraceSamplingRatio != v.Expected.TraceSamplingRatio {
			t.Errorf("ResolveTelemetrySettings(%v) = %+v, want %+v", v.Input, got, v.Expected)
		}
	}
}

func TestShouldEmitLogVectors(t *testing.T) {
	var vectors []struct {
		Input struct {
			Level            string  `json:"level"`
			SessionID        string  `json:"sessionId"`
			TraceSampled     *bool   `json:"traceSampled"`
			Enabled          bool    `json:"enabled"`
			MinimumLevel     string  `json:"minimumLevel"`
			LogSamplingRatio float64 `json:"logSamplingRatio"`
		} `json:"input"`
		Expected bool `json:"expected"`
	}
	section(t, "shouldEmitLog", &vectors)
	for _, v := range vectors {
		minimum, ok := ParseLevel(v.Input.MinimumLevel)
		if !ok {
			t.Fatalf("corpus names an unknown canonical level: %s", v.Input.MinimumLevel)
		}
		got := ShouldEmitLog(LogSamplingInput{
			Level:            v.Input.Level,
			SessionID:        v.Input.SessionID,
			TraceSampled:     v.Input.TraceSampled,
			Enabled:          v.Input.Enabled,
			MinimumLevel:     minimum,
			LogSamplingRatio: v.Input.LogSamplingRatio,
		})
		if got != v.Expected {
			t.Errorf("ShouldEmitLog(%+v) = %v, want %v", v.Input, got, v.Expected)
		}
	}
}
