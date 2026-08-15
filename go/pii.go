package observability

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"regexp"
	"strings"
	"sync"
)

// PII scrubbing — applied to message strings, breadcrumb messages, and headers
// before transport. Port of rust/observability/src/pii.rs; the semantics are
// identical across the five SDKs.
//
// Two classes, handled differently on purpose:
//
//   - Credentials (Bearer …, password=, token/api_key/secret=, sk-…) are
//     DROPPED. A hash of a live token is still a token oracle, and there is no
//     correlation value in a secret.
//   - Personal identifiers (email, phone, street address) are HASHED, not
//     dropped: a@b.com -> [email:9f2a41c8]. That keeps the one question worth
//     asking — "are these two spans the same person?" — answerable while
//     storing nothing reversible.
//
// The hash is HMAC-SHA256, keyed, not a bare digest: emails and phone numbers
// are a small enumerable space that a rainbow table reverses in seconds. The
// org id is mixed into the HMAC message, so identical PII hashes DIFFERENTLY in
// different orgs.
//
// The key and the org salt are load-bearing and must not rotate casually.
// Rotating either silently breaks correlation with every previously stored
// hash. Supply the key once at startup via SMOOAI_OBSERVABILITY_PII_HASH_KEY
// (read by Bootstrap) or SetPiiHashKey. With no key configured, personal
// identifiers are fully redacted ([email:redacted]) rather than hashed under a
// guessable one — fail safe, never fail open.

// Go's regexp (RE2) has no backreferences, so the TS pattern that reused `$&`
// with a JS `.replace` callback is reimplemented with a capture group +
// replacement template. The match semantics are preserved.

type piiPattern struct {
	re          *regexp.Regexp
	replacement string
}

// credentialPatterns are matched FIRST so a personal identifier sitting inside
// a secret (token=a@b.com) is dropped with the secret rather than surviving as
// a hash.
var credentialPatterns = []piiPattern{
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

// PiiKind is the class of personal identifier a match represents. It drives both
// the visible prefix in the output token and the normalization applied before
// hashing, so "(415) 555-0142" and "415-555-0142" correlate.
type PiiKind string

const (
	PiiEmail   PiiKind = "email"
	PiiPhone   PiiKind = "phone"
	PiiAddress PiiKind = "address"
)

// Label is the prefix that stays visible in the scrubbed output.
func (k PiiKind) Label() string { return string(k) }

var nonDigit = regexp.MustCompile(`[^0-9]`)

func (k PiiKind) normalize(raw string) string {
	switch k {
	case PiiEmail:
		return strings.ToLower(strings.TrimSpace(raw))
	case PiiPhone:
		// Digits only: formatting must not fork the hash.
		return nonDigit.ReplaceAllString(raw, "")
	case PiiAddress:
		// Case-fold and collapse runs of whitespace.
		return strings.ToLower(strings.Join(strings.Fields(raw), " "))
	default:
		return raw
	}
}

// personalPatterns are hashed, not dropped. Order matters only in that these
// all run after credentialPatterns.
var personalPatterns = []struct {
	kind PiiKind
	re   *regexp.Regexp
}{
	{PiiEmail, regexp.MustCompile(`(?i)\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+\b`)},
	// Phone: optional country code / area code, then the 3-4 local pair. A
	// separator is REQUIRED so bare digit runs (ids, timestamps, amounts) don't
	// get eaten.
	{PiiPhone, regexp.MustCompile(`(?:\+\d{1,3}[ .-]?)?(?:\(\d{3}\)[ .-]?|\b\d{3}[ .-])?\b\d{3}[ .-]\d{4}\b`)},
	// US-style street address: house number, 1-3 words, a street suffix.
	{PiiAddress, regexp.MustCompile(`(?i)\b\d{1,6}\s+(?:[A-Za-z0-9.'-]+\s+){0,3}(?:street|st|avenue|ave|road|rd|boulevard|blvd|lane|ln|drive|dr|court|ct|way|terrace|ter|place|pl|circle|cir|highway|hwy|parkway|pkwy|square|sq)\b\.?`)},
}

// sensitiveHeaders are header names whose values are fully redacted.
var sensitiveHeaders = map[string]struct{}{
	"authorization": {},
	"cookie":        {},
	"set-cookie":    {},
	"x-api-key":     {},
	"x-auth-token":  {},
}

// hashHexLen is how many hex characters are kept from the HMAC. Long enough
// that collisions are rare across an org's traces, short enough to read in a
// span attribute.
const hashHexLen = 8

var (
	piiKeyMu sync.RWMutex
	piiKey   []byte
)

// SetPiiHashKey installs the process-wide HMAC key used to hash personal
// identifiers. Idempotent and set-once: returns false if a key was already
// installed (the existing key is kept — a mid-process rotation would silently
// fork every correlation) or if key is empty. Bootstrap calls this from
// SMOOAI_OBSERVABILITY_PII_HASH_KEY.
func SetPiiHashKey(key []byte) bool {
	if len(key) == 0 {
		return false
	}
	piiKeyMu.Lock()
	defer piiKeyMu.Unlock()
	if piiKey != nil {
		return false
	}
	piiKey = append([]byte(nil), key...)
	return true
}

func piiHashKey() []byte {
	piiKeyMu.RLock()
	defer piiKeyMu.RUnlock()
	return piiKey
}

// PiiToken hashes one known-personal value into its scrubbed token — the same
// token ScrubStringForOrg would have written. This is how a UI search box finds
// stored hashes: hash the typed term with the same org and match.
//
// Returns "[<kind>:redacted]" when no key is installed.
func PiiToken(kind PiiKind, raw, orgID string) string {
	return tokenWithKey(kind, raw, orgID, piiHashKey())
}

func tokenWithKey(kind PiiKind, raw, orgID string, key []byte) string {
	if len(key) == 0 {
		return "[" + kind.Label() + ":redacted]"
	}
	// orgID in the HMAC message IS the per-org salt. The kind is in there too so
	// a phone and an address that normalize alike can't collide.
	mac := hmac.New(sha256.New, key)
	mac.Write([]byte(orgID))
	mac.Write([]byte{0})
	mac.Write([]byte(kind.Label()))
	mac.Write([]byte{0})
	mac.Write([]byte(kind.normalize(raw)))
	return "[" + kind.Label() + ":" + hex.EncodeToString(mac.Sum(nil))[:hashHexLen] + "]"
}

// ScrubString scrubs a free-form string with no org context — credentials
// dropped, personal identifiers hashed under the empty org salt. Prefer
// ScrubStringForOrg wherever an org id is in hand, so hashes can't be
// correlated across tenants.
func ScrubString(input string) string {
	return scrubWithKey(input, "", piiHashKey())
}

// ScrubStringForOrg scrubs a free-form string, salting personal-identifier
// hashes with orgID.
func ScrubStringForOrg(input, orgID string) string {
	return scrubWithKey(input, orgID, piiHashKey())
}

func scrubWithKey(input, orgID string, key []byte) string {
	out := input
	for _, p := range credentialPatterns {
		out = p.re.ReplaceAllString(out, p.replacement)
	}
	for _, p := range personalPatterns {
		kind := p.kind
		out = p.re.ReplaceAllStringFunc(out, func(match string) string {
			return tokenWithKey(kind, match, orgID, key)
		})
	}
	return out
}

// ScrubHeaders fully redacts sensitive header values and scrubs the rest.
// Returns nil for a nil map (matches the TS undefined passthrough).
func ScrubHeaders(headers map[string]string) map[string]string {
	return ScrubHeadersForOrg(headers, "")
}

// ScrubHeadersForOrg is ScrubHeaders with an org salt for the
// personal-identifier hashes.
func ScrubHeadersForOrg(headers map[string]string, orgID string) map[string]string {
	if headers == nil {
		return nil
	}
	out := make(map[string]string, len(headers))
	for k, v := range headers {
		if _, ok := sensitiveHeaders[strings.ToLower(k)]; ok {
			out[k] = "[redacted]"
		} else {
			out[k] = ScrubStringForOrg(v, orgID)
		}
	}
	return out
}
