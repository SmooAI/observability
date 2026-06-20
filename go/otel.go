package observability

import (
	"context"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetrichttp"
	"go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp"
	"go.opentelemetry.io/otel/propagation"
	"go.opentelemetry.io/otel/sdk/metric"
	"go.opentelemetry.io/otel/sdk/resource"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
	semconv "go.opentelemetry.io/otel/semconv/v1.41.0"
	"go.opentelemetry.io/otel/trace"
)

// OTel SDK bootstrap — the Go equivalent of setup-otel-sdk.ts. Initializes a
// TracerProvider + MeterProvider with OTLP/HTTP export, registers them as the
// global providers, and returns a handle with Flush / Shutdown. Idempotent: a
// second call returns the same handle.
//
// Auth: instead of the TS custom SpanExporter, Go injects the per-request
// Bearer via a custom http.RoundTripper (authRoundTripper) handed to the OTLP
// HTTP exporters with WithHTTPClient. The RoundTripper pulls a fresh token from
// the TokenProvider on every request and retries once on 401 — no header
// snapshot, no expiry drift (the SMOODEV-1206 fix, achieved at the transport
// layer).

// SetupOtelOptions configures SetupOtelSDK.
type SetupOtelOptions struct {
	// ServiceName surfaced as service.name on every span/metric.
	ServiceName string
	// Environment ('production', 'staging', ...). Surfaced as
	// deployment.environment.name.
	Environment string
	// Release identifier — surfaced as service.version.
	Release string
	// TracesEndpoint is the full OTLP/HTTP traces URL (e.g.
	// https://api.smoo.ai/v1/traces). Falls back to env vars when empty.
	TracesEndpoint string
	// MetricsEndpoint is the full OTLP/HTTP metrics URL. Falls back to env.
	MetricsEndpoint string
	// Headers are static headers merged onto every export request.
	Headers map[string]string
	// TokenProvider, when set, injects a fresh Bearer per export request.
	TokenProvider *TokenProvider
	// MetricExportInterval — default 30s.
	MetricExportInterval time.Duration
	// SkipStart constructs the providers without registering them globally
	// (test seam).
	SkipStart bool
	// HTTPClient overrides the exporter transport base client (test seam).
	HTTPClient *http.Client
}

// OtelSDKHandle is returned by SetupOtelSDK for lifecycle control.
type OtelSDKHandle struct {
	TracerProvider *sdktrace.TracerProvider
	MeterProvider  *metric.MeterProvider
}

// Flush force-flushes spans and metrics, bounded by timeoutMs. Best-effort.
func (h *OtelSDKHandle) Flush(ctx context.Context, timeout time.Duration) {
	defer recoverSilently()
	if timeout <= 0 {
		timeout = 2 * time.Second
	}
	fctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	if h.TracerProvider != nil {
		_ = h.TracerProvider.ForceFlush(fctx)
	}
	if h.MeterProvider != nil {
		_ = h.MeterProvider.ForceFlush(fctx)
	}
}

// Shutdown drains and closes the pipelines. Idempotent.
func (h *OtelSDKHandle) Shutdown(ctx context.Context) {
	defer recoverSilently()
	if h.TracerProvider != nil {
		_ = h.TracerProvider.Shutdown(ctx)
	}
	if h.MeterProvider != nil {
		_ = h.MeterProvider.Shutdown(ctx)
	}
	otelInstallMu.Lock()
	otelInstalled = nil
	otelInstallMu.Unlock()
}

var (
	otelInstallMu sync.Mutex
	otelInstalled *OtelSDKHandle
)

// SetupOtelSDK initializes and registers the OTel providers. Idempotent. Never
// panics — on failure it returns a handle with nil providers and the global
// OTel API quietly no-ops.
func SetupOtelSDK(ctx context.Context, opts SetupOtelOptions) *OtelSDKHandle {
	otelInstallMu.Lock()
	defer otelInstallMu.Unlock()
	if otelInstalled != nil {
		return otelInstalled
	}

	handle := &OtelSDKHandle{}
	defer func() {
		if r := recover(); r != nil {
			// Leave whatever was constructed; never propagate.
			otelInstalled = handle
		}
	}()

	traceEndpoint := firstNonEmpty(opts.TracesEndpoint,
		os.Getenv("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
		os.Getenv("OTEL_EXPORTER_OTLP_ENDPOINT"))
	metricEndpoint := firstNonEmpty(opts.MetricsEndpoint,
		os.Getenv("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"),
		os.Getenv("OTEL_EXPORTER_OTLP_ENDPOINT"))

	res := buildResource(opts)

	// --- Traces ---
	if traceEndpoint != "" {
		traceOpts := []otlptracehttp.Option{otlptracehttp.WithEndpointURL(traceEndpoint)}
		if len(opts.Headers) > 0 {
			traceOpts = append(traceOpts, otlptracehttp.WithHeaders(opts.Headers))
		}
		if client := buildHTTPClient(opts); client != nil {
			traceOpts = append(traceOpts, otlptracehttp.WithHTTPClient(client))
		}
		if exp, err := otlptracehttp.New(ctx, traceOpts...); err == nil {
			tp := sdktrace.NewTracerProvider(
				sdktrace.WithBatcher(exp),
				sdktrace.WithResource(res),
			)
			handle.TracerProvider = tp
		}
	}

	// --- Metrics ---
	if metricEndpoint != "" {
		metricOpts := []otlpmetrichttp.Option{otlpmetrichttp.WithEndpointURL(metricEndpoint)}
		if len(opts.Headers) > 0 {
			metricOpts = append(metricOpts, otlpmetrichttp.WithHeaders(opts.Headers))
		}
		if client := buildHTTPClient(opts); client != nil {
			metricOpts = append(metricOpts, otlpmetrichttp.WithHTTPClient(client))
		}
		if exp, err := otlpmetrichttp.New(ctx, metricOpts...); err == nil {
			interval := opts.MetricExportInterval
			if interval <= 0 {
				interval = 30 * time.Second
			}
			reader := metric.NewPeriodicReader(exp, metric.WithInterval(interval))
			mp := metric.NewMeterProvider(
				metric.WithReader(reader),
				metric.WithResource(res),
			)
			handle.MeterProvider = mp
		}
	}

	if !opts.SkipStart {
		if handle.TracerProvider != nil {
			otel.SetTracerProvider(handle.TracerProvider)
		}
		if handle.MeterProvider != nil {
			otel.SetMeterProvider(handle.MeterProvider)
		}
		otel.SetTextMapPropagator(propagation.NewCompositeTextMapPropagator(
			propagation.TraceContext{}, propagation.Baggage{}))
	}

	otelInstalled = handle
	return handle
}

func buildResource(opts SetupOtelOptions) *resource.Resource {
	var kvs []attribute.KeyValue
	if opts.ServiceName != "" {
		kvs = append(kvs, semconv.ServiceNameKey.String(opts.ServiceName))
	}
	if opts.Release != "" {
		kvs = append(kvs, semconv.ServiceVersionKey.String(opts.Release))
	}
	if opts.Environment != "" {
		kvs = append(kvs, semconv.DeploymentEnvironmentNameKey.String(opts.Environment))
	}
	r, err := resource.Merge(resource.Default(), resource.NewWithAttributes(semconv.SchemaURL, kvs...))
	if err != nil || r == nil {
		return resource.Default()
	}
	return r
}

// buildHTTPClient returns the custom-auth HTTP client when a TokenProvider is
// set, the caller's override, or nil to let the exporter use its default.
func buildHTTPClient(opts SetupOtelOptions) *http.Client {
	if opts.TokenProvider == nil && opts.HTTPClient == nil {
		return nil
	}
	base := opts.HTTPClient
	if base == nil {
		base = &http.Client{Timeout: 30 * time.Second}
	}
	if opts.TokenProvider == nil {
		return base
	}
	baseRT := base.Transport
	if baseRT == nil {
		baseRT = http.DefaultTransport
	}
	return &http.Client{
		Timeout: base.Timeout,
		Transport: &authRoundTripper{
			base:          baseRT,
			tokenProvider: opts.TokenProvider,
		},
	}
}

// authRoundTripper injects a fresh Bearer on every request and retries once on
// 401 (re-minting the token). This is the Go realization of the TS
// AuthInjectingExporter — done at the transport layer instead of a custom
// SpanExporter, so the stock OTLP HTTP exporters can be used unchanged.
type authRoundTripper struct {
	base          http.RoundTripper
	tokenProvider *TokenProvider
}

func (a *authRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	token, err := a.tokenProvider.AccessToken(req.Context())
	if err != nil {
		return nil, err
	}
	// Clone so we don't mutate the caller's request when retrying.
	r1 := cloneRequest(req)
	r1.Header.Set("Authorization", "Bearer "+token)
	resp, err := a.base.RoundTrip(r1)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode == http.StatusUnauthorized {
		resp.Body.Close()
		a.tokenProvider.Invalidate()
		fresh, terr := a.tokenProvider.AccessToken(req.Context())
		if terr != nil {
			return nil, terr
		}
		r2 := cloneRequest(req)
		r2.Header.Set("Authorization", "Bearer "+fresh)
		return a.base.RoundTrip(r2)
	}
	return resp, nil
}

func cloneRequest(req *http.Request) *http.Request {
	r := req.Clone(req.Context())
	if req.Body != nil && req.GetBody != nil {
		if body, err := req.GetBody(); err == nil {
			r.Body = body
		}
	}
	return r
}

// OtelCorrelation is the read-only view of the active span context. Mirrors
// read-otel-context.ts.
type OtelCorrelation struct {
	TraceID string
	SpanID  string
	Sampled bool
}

// ReadOtelCorrelation reads the active span context from ctx into the Smoo
// correlation shape. Returns a zero value when no span is recording.
func ReadOtelCorrelation(ctx context.Context) OtelCorrelation {
	sc := trace.SpanContextFromContext(ctx)
	if !sc.IsValid() {
		return OtelCorrelation{}
	}
	return OtelCorrelation{
		TraceID: sc.TraceID().String(),
		SpanID:  sc.SpanID().String(),
		Sampled: sc.IsSampled(),
	}
}

func resetOtelSDK() {
	otelInstallMu.Lock()
	defer otelInstallMu.Unlock()
	otelInstalled = nil
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if strings.TrimSpace(v) != "" {
			return v
		}
	}
	return ""
}
