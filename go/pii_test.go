package observability

import "testing"

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
