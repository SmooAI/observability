package observability

import (
	"regexp"
	"strings"
)

// PII scrubbing — applied to message strings, breadcrumb messages, and headers
// before transport. Ports the exact patterns from the TS reference SDK
// (packages/core/src/pii.ts). Stays opinionated and minimal; callers can extend
// in a BeforeSend hook.

// Go's regexp (RE2) has no backreferences, so the TS pattern that reused `$&`
// with a JS `.replace` callback is reimplemented with a capture group +
// replacement template. The match semantics are preserved.

type piiPattern struct {
	re          *regexp.Regexp
	replacement string
}

var piiPatterns = []piiPattern{
	// Bearer tokens.
	{regexp.MustCompile(`(?i)Bearer\s+[A-Za-z0-9._-]+`), "Bearer [redacted]"},
	// password=... / passwd: ... / pwd = ...
	{regexp.MustCompile(`(?i)\b(?:password|passwd|pwd)["']?\s*[:=]\s*["']?[^"'&\s]+`), "password=[redacted]"},
	// token / api_key / apikey / secret = ... — keep the key, redact the value.
	// TS used `$&`.replace(/=.*/, '=[redacted]'); the equivalent here captures
	// the key+separator and rewrites only the value.
	{regexp.MustCompile(`(?i)\b((?:token|api[-_]?key|apikey|secret)["']?\s*[:=]\s*)["']?[^"'&\s]+`), "${1}[redacted]"},
	// OpenAI-style sk- keys.
	{regexp.MustCompile(`sk-[A-Za-z0-9]{20,}`), "sk-[redacted]"},
}

// sensitiveHeaders are header names whose values are fully redacted.
var sensitiveHeaders = map[string]struct{}{
	"authorization": {},
	"cookie":        {},
	"set-cookie":    {},
	"x-api-key":     {},
	"x-auth-token":  {},
}

// ScrubString applies the PII patterns to a single string.
func ScrubString(input string) string {
	out := input
	for _, p := range piiPatterns {
		out = p.re.ReplaceAllString(out, p.replacement)
	}
	return out
}

// ScrubHeaders fully redacts sensitive header values and scrubs the rest.
// Returns nil for a nil map (matches the TS undefined passthrough).
func ScrubHeaders(headers map[string]string) map[string]string {
	if headers == nil {
		return nil
	}
	out := make(map[string]string, len(headers))
	for k, v := range headers {
		if _, ok := sensitiveHeaders[strings.ToLower(k)]; ok {
			out[k] = "[redacted]"
		} else {
			out[k] = ScrubString(v)
		}
	}
	return out
}
