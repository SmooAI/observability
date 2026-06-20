package observability

import (
	"context"
	"os"
	"strings"
	"sync"
)

// One-call, env-driven bootstrap — the Go port of bootstrap/index.ts. Reads the
// same SMOOAI_OBSERVABILITY_* env vars, mints an M2M token (or uses a pre-minted
// one), sets up the OTel SDK with per-request auth, initializes the Client, and
// registers the OTel-native capture path. Idempotent and panic-safe: on any
// failure it logs to stderr and the host keeps running.
//
// Required: SMOOAI_OBSERVABILITY_ENDPOINT (base URL; /v1/traces + /v1/metrics
// are appended) or the per-signal OTEL_EXPORTER_OTLP_*_ENDPOINT vars.
//
// Auth (pick one; pre-minted JWT wins):
//   - SMOOAI_OBSERVABILITY_TOKEN — pre-minted Bearer JWT.
//   - SMOOAI_OBSERVABILITY_AUTH_URL + _CLIENT_ID + _CLIENT_SECRET —
//     client_credentials grant, refreshed per-request by the exporter.
//
// Optional: _SERVICE_NAME (default "smoo-service"), _ENVIRONMENT
// (default STAGE / GO_ENV / "unknown"), _RELEASE (default GIT_SHA / "dev"),
// _DISABLED ("1"/"true" to skip).

// BootstrapResult reports what the bootstrap did.
type BootstrapResult struct {
	// Installed is false when disabled or already bootstrapped.
	Installed bool
	// Otel is the SDK handle (nil if init failed or was skipped).
	Otel *OtelSDKHandle
}

// BootstrapEnv mirrors the resolved env config; overrides for testing.
type BootstrapEnv struct {
	Endpoint        string
	TracesEndpoint  string
	MetricsEndpoint string
	Token           string
	AuthURL         string
	ClientID        string
	ClientSecret    string
	ServiceName     string
	Environment     string
	Release         string
	Disabled        bool
	// DSN, when set, also wires the webhook HTTP transport so captures fan out
	// to both OTel and the Errors dashboard (SMOODEV-1148 parity). Resolved
	// from OBSERVABILITY_DSN when not overridden.
	DSN string
}

var (
	bootstrapMu     sync.Mutex
	bootstrapResult *BootstrapResult
)

// Bootstrap runs the env-driven bootstrap on the default client. Idempotent.
// Never panics. Pass overrides to substitute values (tests / advanced callers).
func Bootstrap(ctx context.Context, overrides *BootstrapEnv) BootstrapResult {
	bootstrapMu.Lock()
	defer bootstrapMu.Unlock()
	if bootstrapResult != nil {
		return *bootstrapResult
	}

	result := BootstrapResult{}
	defer func() {
		if r := recover(); r != nil {
			warn("bootstrap: panic recovered; SDK disabled")
			result = BootstrapResult{}
			bootstrapResult = &result
		}
	}()

	env := resolveEnv(overrides)

	if env.Disabled {
		result = BootstrapResult{Installed: false}
		bootstrapResult = &result
		return result
	}

	headers := map[string]string{}
	var tokenProvider *TokenProvider

	switch {
	case env.Token != "":
		headers["Authorization"] = "Bearer " + env.Token
	case env.AuthURL != "" && env.ClientID != "" && env.ClientSecret != "":
		tp, err := NewTokenProvider(TokenProviderOptions{
			AuthURL:      env.AuthURL,
			ClientID:     env.ClientID,
			ClientSecret: env.ClientSecret,
		})
		if err != nil {
			warn("bootstrap: token provider construction failed: " + err.Error())
		} else {
			tokenProvider = tp
			// Warm-up mint so the first export doesn't pay the round-trip.
			if _, mErr := tp.AccessToken(ctx); mErr != nil {
				warn("bootstrap: initial token mint failed; exports will retry: " + mErr.Error())
			}
		}
	default:
		warn("bootstrap: no auth configured; OTLP exports will be unauthenticated")
	}

	traceEndpoint := env.TracesEndpoint
	if traceEndpoint == "" && env.Endpoint != "" {
		traceEndpoint = strings.TrimRight(env.Endpoint, "/") + "/v1/traces"
	}
	metricEndpoint := env.MetricsEndpoint
	if metricEndpoint == "" && env.Endpoint != "" {
		metricEndpoint = strings.TrimRight(env.Endpoint, "/") + "/v1/metrics"
	}

	otelHandle := SetupOtelSDK(ctx, SetupOtelOptions{
		ServiceName:     env.ServiceName,
		Environment:     env.Environment,
		Release:         env.Release,
		TracesEndpoint:  traceEndpoint,
		MetricsEndpoint: metricEndpoint,
		Headers:         headers,
		TokenProvider:   tokenProvider,
	})

	Default.Init(ClientOptions{
		DSN:         env.DSN,
		Environment: env.Environment,
		Release:     env.Release,
	})
	RegisterOtelCapture(Default, "")

	// SMOODEV-1148: when a DSN is set, also fan out to the webhook transport.
	if env.DSN != "" {
		t := newTransportFromClientOptions(*Default.Options())
		Default.RegisterTransport(func(batch []ObservabilityEvent) {
			for _, e := range batch {
				t.Enqueue(e)
			}
		})
	}

	result = BootstrapResult{Installed: true, Otel: otelHandle}
	bootstrapResult = &result
	return result
}

func resolveEnv(o *BootstrapEnv) BootstrapEnv {
	if o == nil {
		o = &BootstrapEnv{}
	}
	pick := func(override, env string) string {
		if override != "" {
			return override
		}
		return os.Getenv(env)
	}
	return BootstrapEnv{
		Endpoint:        pick(o.Endpoint, "SMOOAI_OBSERVABILITY_ENDPOINT"),
		TracesEndpoint:  pick(o.TracesEndpoint, "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
		MetricsEndpoint: pick(o.MetricsEndpoint, "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"),
		Token:           pick(o.Token, "SMOOAI_OBSERVABILITY_TOKEN"),
		AuthURL:         pick(o.AuthURL, "SMOOAI_OBSERVABILITY_AUTH_URL"),
		ClientID:        pick(o.ClientID, "SMOOAI_OBSERVABILITY_CLIENT_ID"),
		ClientSecret:    pick(o.ClientSecret, "SMOOAI_OBSERVABILITY_CLIENT_SECRET"),
		ServiceName:     firstNonEmpty(o.ServiceName, os.Getenv("SMOOAI_OBSERVABILITY_SERVICE_NAME"), "smoo-service"),
		Environment:     firstNonEmpty(o.Environment, os.Getenv("SMOOAI_OBSERVABILITY_ENVIRONMENT"), os.Getenv("STAGE"), os.Getenv("GO_ENV"), "unknown"),
		Release:         firstNonEmpty(o.Release, os.Getenv("SMOOAI_OBSERVABILITY_RELEASE"), os.Getenv("GIT_SHA"), "dev"),
		Disabled:        o.Disabled || truthy(os.Getenv("SMOOAI_OBSERVABILITY_DISABLED")),
		DSN:             firstNonEmpty(o.DSN, os.Getenv("OBSERVABILITY_DSN")),
	}
}

func truthy(s string) bool {
	s = strings.ToLower(strings.TrimSpace(s))
	return s == "1" || s == "true"
}

func warn(message string) {
	defer recoverSilently()
	_, _ = os.Stderr.WriteString("[observability-go/bootstrap] " + message + "\n")
}

// resetBootstrap is a test seam.
func resetBootstrap() {
	bootstrapMu.Lock()
	defer bootstrapMu.Unlock()
	bootstrapResult = nil
}
