package observability

import (
	"context"
	"io"
	"os"
	"strings"
	"testing"
)

// captureStderr swaps os.Stderr for a pipe for the duration of fn and returns
// what was written. warn() resolves os.Stderr at call time, so this sees it.
func captureStderr(t *testing.T, fn func()) string {
	t.Helper()
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatalf("pipe: %v", err)
	}
	original := os.Stderr
	os.Stderr = w
	defer func() { os.Stderr = original }()

	done := make(chan string, 1)
	go func() {
		b, _ := io.ReadAll(r)
		done <- string(b)
	}()

	fn()
	_ = w.Close()
	return <-done
}

func TestBootstrapDisabled(t *testing.T) {
	resetBootstrap()
	defer resetBootstrap()
	res := Bootstrap(context.Background(), &BootstrapEnv{Disabled: true})
	if res.Installed {
		t.Error("disabled bootstrap should not install")
	}
}

func TestBootstrapIdempotent(t *testing.T) {
	resetBootstrap()
	resetOtelSDK()
	defer resetBootstrap()
	defer resetOtelSDK()

	r1 := Bootstrap(context.Background(), &BootstrapEnv{ServiceName: "svc"})
	r2 := Bootstrap(context.Background(), &BootstrapEnv{ServiceName: "other"})
	if r1.Installed != r2.Installed {
		t.Error("bootstrap not idempotent")
	}
}

func TestBootstrapInstallsClientAndCapture(t *testing.T) {
	resetBootstrap()
	resetOtelSDK()
	defer resetBootstrap()
	defer resetOtelSDK()

	// SetupOtelSDK falls back to these env vars, so "no endpoint" has to mean
	// no endpoint from ANY source or the Exporting assertion below is
	// environment-dependent. t.Setenv restores them after the test.
	for _, key := range []string{
		"OTEL_EXPORTER_OTLP_ENDPOINT",
		"OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
		"OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
		"OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
		"SMOOAI_OBSERVABILITY_ENDPOINT",
	} {
		t.Setenv(key, "")
	}

	// No endpoint / no auth — bootstrap should still install the Client and the
	// OTel-native capture path without panicking.
	var res BootstrapResult
	stderr := captureStderr(t, func() {
		res = Bootstrap(context.Background(), &BootstrapEnv{
			ServiceName: "svc",
			Environment: "test",
			Release:     "r1",
		})
	})
	if !strings.Contains(stderr, "NO OTLP ENDPOINT CONFIGURED") {
		t.Errorf("no-endpoint bootstrap must warn loudly; stderr was: %q", stderr)
	}
	if !strings.Contains(stderr, "SMOOAI_OBSERVABILITY_DISABLED=true") {
		t.Errorf("warning must offer the explicit-disable var; stderr was: %q", stderr)
	}
	if !res.Installed {
		t.Fatal("bootstrap did not install")
	}
	// …but it is NOT exporting, and the result now says so. This assertion is
	// the whole point: Installed alone used to be the only signal, and it reads
	// as "everything is fine" while nothing leaves the process.
	if res.Exporting {
		t.Error("no endpoint configured must report Exporting = false")
	}
	if !Default.IsInitialized() {
		t.Error("default client not initialized by bootstrap")
	}
	opts := Default.Options()
	if opts.Environment != "test" || opts.Release != "r1" {
		t.Errorf("client options wrong: %+v", opts)
	}
	// captureHandler should be registered (OTel-native path).
	Default.mu.RLock()
	hasHandler := Default.captureHandler != nil
	Default.mu.RUnlock()
	if !hasHandler {
		t.Error("OTel capture handler not registered")
	}
}

func TestBootstrapWiresWebhookTransportWhenDSN(t *testing.T) {
	resetBootstrap()
	resetOtelSDK()
	defer resetBootstrap()
	defer resetOtelSDK()

	res := Bootstrap(context.Background(), &BootstrapEnv{
		ServiceName: "svc",
		DSN:         "https://example.test/dsn",
	})
	if !res.Installed {
		t.Fatal("not installed")
	}
	Default.mu.RLock()
	hasTransport := Default.transport != nil
	Default.mu.RUnlock()
	if !hasTransport {
		t.Error("webhook transport not wired despite DSN")
	}
}

// The inverse of the no-endpoint case: with an endpoint configured the result
// must claim it IS exporting. Without both halves asserted, a regression that
// hard-codes either value passes.
func TestBootstrapExportingWhenEndpointConfigured(t *testing.T) {
	resetBootstrap()
	resetOtelSDK()
	defer resetBootstrap()
	defer resetOtelSDK()

	res := Bootstrap(context.Background(), &BootstrapEnv{
		ServiceName: "svc",
		Endpoint:    "https://collector.example.com",
		Token:       "pre-minted",
	})
	if !res.Installed {
		t.Fatal("bootstrap did not install")
	}
	if !res.Exporting {
		t.Error("an endpoint was configured, so Exporting must be true")
	}
	if res.Otel == nil || res.Otel.TracerProvider == nil {
		t.Error("expected a tracer provider when an endpoint is configured")
	}
}

func TestResolveEnvReadsPiiHashKey(t *testing.T) {
	t.Setenv("SMOOAI_OBSERVABILITY_PII_HASH_KEY", "env-supplied-key")
	if got := resolveEnv(nil).PiiHashKey; got != "env-supplied-key" {
		t.Errorf("PiiHashKey = %q, want the env value", got)
	}
	// An explicit override still wins.
	if got := resolveEnv(&BootstrapEnv{PiiHashKey: "override"}).PiiHashKey; got != "override" {
		t.Errorf("override ignored: %q", got)
	}
}
