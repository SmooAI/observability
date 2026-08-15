package observability

import (
	"strings"
	"testing"
)

func TestScrubString(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want string
	}{
		{"bearer", "Authorization: Bearer abc.def-ghi_123", "Authorization: Bearer [redacted]"},
		{"password", `password="hunter2"`, `password=[redacted]"`},
		{"pwd colon", "pwd: secretval", "password=[redacted]"},
		{"api key", `api_key=sk_live_999`, `api_key=[redacted]`},
		{"token equals", "token=xyz123", "token=[redacted]"},
		{"sk key", "sk-ABCDEFGHIJKLMNOPQRSTUVWX", "sk-[redacted]"},
		{"clean", "nothing to see here", "nothing to see here"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got := ScrubString(c.in)
			if got != c.want {
				t.Errorf("ScrubString(%q) = %q, want %q", c.in, got, c.want)
			}
		})
	}
}

func TestScrubHeaders(t *testing.T) {
	if ScrubHeaders(nil) != nil {
		t.Fatal("nil headers should pass through as nil")
	}
	in := map[string]string{
		"Authorization": "Bearer secret",
		"Cookie":        "session=abc",
		"X-Api-Key":     "key123",
		"User-Agent":    "Bearer notatoken-but-scrubbed",
		"Content-Type":  "application/json",
	}
	out := ScrubHeaders(in)
	if out["Authorization"] != "[redacted]" {
		t.Errorf("Authorization not redacted: %q", out["Authorization"])
	}
	if out["Cookie"] != "[redacted]" {
		t.Errorf("Cookie not redacted: %q", out["Cookie"])
	}
	if out["X-Api-Key"] != "[redacted]" {
		t.Errorf("X-Api-Key not redacted: %q", out["X-Api-Key"])
	}
	// Non-sensitive header still gets string-scrubbed.
	if out["User-Agent"] != "Bearer [redacted]" {
		t.Errorf("User-Agent value not scrubbed: %q", out["User-Agent"])
	}
	if out["Content-Type"] != "application/json" {
		t.Errorf("Content-Type mangled: %q", out["Content-Type"])
	}
}

// ---- PII hashing parity with rust/observability/src/pii.rs -----------------
//
// These drive scrubWithKey / tokenWithKey rather than installing the
// process-wide key: SetPiiHashKey is set-once and `go test` runs the package in
// one process, so a global write would make the suite order-dependent.

const (
	testKey      = "test-hmac-key-not-a-real-secret"
	testOtherKey = "a-different-test-hmac-key"
)

func scrubT(input, org string) string {
	return scrubWithKey(input, org, []byte(testKey))
}

func TestCredentialsAreDroppedNotHashed(t *testing.T) {
	// A live token must never become a correlatable handle — hashing one still
	// yields an oracle you can test candidate tokens against.
	s := scrubT("Bearer abc.def-ghi_123 password=hunter2 sk-ABCDEFGHIJKLMNOPQRSTUVWX", "org-1")
	for _, want := range []string{"Bearer [redacted]", "password=[redacted]", "sk-[redacted]"} {
		if !strings.Contains(s, want) {
			t.Errorf("missing %q in %q", want, s)
		}
	}
	for _, bad := range []string{"[token:", "[credential:", "[email:", "[phone:"} {
		if strings.Contains(s, bad) {
			t.Errorf("credential was hashed (%q) in %q", bad, s)
		}
	}
	// And a personal identifier hiding inside a secret goes with it.
	s2 := scrubT("token=a@b.com", "org-1")
	if strings.Contains(s2, "a@b.com") || strings.Contains(s2, "[email:") {
		t.Errorf("PII inside a secret survived: %q", s2)
	}
}

func TestHashesEmailsKeepingTheTypePrefix(t *testing.T) {
	s := scrubT("contact me at Alice@Example.com please", "org-1")
	if strings.Contains(strings.ToLower(s), "alice@example.com") {
		t.Fatalf("raw email survived: %q", s)
	}
	if !strings.HasPrefix(s, "contact me at [email:") || !strings.HasSuffix(s, "] please") {
		t.Errorf("type prefix not preserved in place: %q", s)
	}
}

func TestHashesPhoneNumbers(t *testing.T) {
	for _, raw := range []string{"555-0142", "(415) 555-0142", "+1 415-555-0142"} {
		s := scrubT("call "+raw+" today", "org-1")
		if !strings.Contains(s, "[phone:") {
			t.Errorf("%q not hashed: %q", raw, s)
		}
		if strings.Contains(s, "0142") {
			t.Errorf("%q leaked digits: %q", raw, s)
		}
	}
}

func TestHashesStreetAddresses(t *testing.T) {
	s := scrubT("ship to 1600 Pennsylvania Ave, Washington", "org-1")
	if !strings.Contains(s, "[address:") || strings.Contains(s, "Pennsylvania") {
		t.Errorf("address not hashed: %q", s)
	}
}

func TestSameValueSameOrgIsStable(t *testing.T) {
	if a, b := scrubT("a@b.com", "org-1"), scrubT("a@b.com", "org-1"); a != b {
		t.Errorf("not deterministic: %q != %q", a, b)
	}
	// …and correlation survives formatting differences in phones.
	if a, b := scrubT("(415) 555-0142", "org-1"), scrubT("415-555-0142", "org-1"); a != b {
		t.Errorf("normalization missing: %q != %q", a, b)
	}
}

func TestSameValueDifferentOrgHashesDifferently(t *testing.T) {
	a, b := scrubT("a@b.com", "org-1"), scrubT("a@b.com", "org-2")
	if a == b {
		t.Errorf("per-org salt missing: %s == %s", a, b)
	}
	if !strings.HasPrefix(a, "[email:") || !strings.HasPrefix(b, "[email:") {
		t.Errorf("prefix lost: %q %q", a, b)
	}
}

func TestDifferentKeyHashesDifferently(t *testing.T) {
	a := scrubWithKey("a@b.com", "org-1", []byte(testKey))
	b := scrubWithKey("a@b.com", "org-1", []byte(testOtherKey))
	if a == b {
		t.Errorf("hash is not keyed: %s == %s", a, b)
	}
}

func TestNoKeyRedactsRatherThanHashing(t *testing.T) {
	// Fail safe: an unkeyed digest of an email is rainbow-tabled instantly.
	s := scrubWithKey("a@b.com and 555-0142", "org-1", nil)
	if !strings.Contains(s, "[email:redacted]") || !strings.Contains(s, "[phone:redacted]") {
		t.Errorf("no-key fallback did not redact: %q", s)
	}
	if strings.Contains(s, "a@b.com") || strings.Contains(s, "0142") {
		t.Errorf("no-key fallback leaked the raw value: %q", s)
	}
}

func TestHashIsShortHex(t *testing.T) {
	tok := tokenWithKey(PiiEmail, "a@b.com", "org-1", []byte(testKey))
	hexPart := strings.TrimSuffix(strings.TrimPrefix(tok, "[email:"), "]")
	if len(hexPart) != hashHexLen {
		t.Errorf("hash length %d, want %d: %q", len(hexPart), hashHexLen, tok)
	}
	for _, c := range hexPart {
		if !strings.ContainsRune("0123456789abcdef", c) {
			t.Errorf("non-hex char %q in %q", c, tok)
		}
	}
}

func TestPiiTokenMatchesWhatScrubbingWrote(t *testing.T) {
	// The searchability contract: hashing a typed query term must produce
	// exactly the token stored in the span.
	scrubbed := scrubT("mail a@b.com now", "org-7")
	term := tokenWithKey(PiiEmail, "A@B.com ", "org-7", []byte(testKey))
	if !strings.Contains(scrubbed, term) {
		t.Errorf("search seam broken: %q does not contain %q", scrubbed, term)
	}
}

func TestOrdinaryNumbersAreNotMistakenForPhones(t *testing.T) {
	// The phone pattern requires a separator before the last 4 digits; ids,
	// versions, dates and amounts must survive intact.
	for _, s := range []string{
		"took 1234 ms",
		"version 1.2.3",
		"2026-08-15T12:00:00Z",
		"total 1234.5678",
		"trace 0af7651916cd43dd8448eb211c80319c",
	} {
		if got := scrubT(s, "org-1"); got != s {
			t.Errorf("false positive on %q -> %q", s, got)
		}
	}
}

func TestSetPiiHashKeyIsSetOnceAndRejectsEmpty(t *testing.T) {
	if SetPiiHashKey(nil) {
		t.Error("empty key should be rejected")
	}
	piiKeyMu.Lock()
	saved := piiKey
	piiKey = nil
	piiKeyMu.Unlock()
	defer func() {
		piiKeyMu.Lock()
		piiKey = saved
		piiKeyMu.Unlock()
	}()

	if !SetPiiHashKey([]byte("first-key")) {
		t.Fatal("first install should succeed")
	}
	if SetPiiHashKey([]byte("second-key")) {
		t.Error("rotation should be refused")
	}
	if string(piiHashKey()) != "first-key" {
		t.Errorf("key was rotated: %q", piiHashKey())
	}
}

func TestScrubHeadersForOrgSaltsValues(t *testing.T) {
	in := map[string]string{"X-Note": "reply to a@b.com"}
	a := ScrubHeadersForOrg(in, "org-1")["X-Note"]
	b := ScrubHeadersForOrg(in, "org-2")["X-Note"]
	if strings.Contains(a, "a@b.com") || strings.Contains(b, "a@b.com") {
		t.Fatalf("raw email in header output: %q / %q", a, b)
	}
	// With no key installed both redact; with one installed they must differ.
	if piiHashKey() != nil && a == b {
		t.Errorf("header hashes not org-salted: %q == %q", a, b)
	}
}

// TestCrossSDKParityVectors pins the exact bytes every SDK must produce.
// Computed independently (python `hmac.new(key, org\0kind\0normalized,
// "sha256")`) and asserted verbatim in all five SDKs. If any SDK's message
// framing, normalization or truncation drifts, exactly one of these breaks.
func TestCrossSDKParityVectors(t *testing.T) {
	cases := []struct {
		kind PiiKind
		raw  string
		org  string
		want string
	}{
		{PiiEmail, "a@b.com", "org-1", "[email:02ea437f]"},
		{PiiEmail, "A@B.COM ", "org-1", "[email:02ea437f]"},
		{PiiEmail, "a@b.com", "org-2", "[email:fd96f7dc]"},
		{PiiEmail, "a@b.com", "", "[email:453b154f]"},
		{PiiPhone, "(415) 555-0142", "org-1", "[phone:415a9aea]"},
		{PiiPhone, "415-555-0142", "org-1", "[phone:415a9aea]"},
		{PiiAddress, "1600  Pennsylvania   Ave", "org-1", "[address:c5351f4a]"},
	}
	for _, c := range cases {
		if got := tokenWithKey(c.kind, c.raw, c.org, []byte(testKey)); got != c.want {
			t.Errorf("tokenWithKey(%s, %q, %q) = %s, want %s", c.kind, c.raw, c.org, got, c.want)
		}
	}
}
