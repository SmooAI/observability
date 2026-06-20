package observability

import (
	"context"
	"testing"
)

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

	// No endpoint / no auth — bootstrap should still install the Client and the
	// OTel-native capture path without panicking.
	res := Bootstrap(context.Background(), &BootstrapEnv{
		ServiceName: "svc",
		Environment: "test",
		Release:     "r1",
	})
	if !res.Installed {
		t.Fatal("bootstrap did not install")
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
