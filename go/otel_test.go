package observability

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"go.opentelemetry.io/otel"
	sdkmetric "go.opentelemetry.io/otel/sdk/metric"
	"go.opentelemetry.io/otel/sdk/metric/metricdata"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
	"go.opentelemetry.io/otel/sdk/trace/tracetest"
	"go.opentelemetry.io/otel/trace"
)

func TestReadOtelCorrelation(t *testing.T) {
	if c := ReadOtelCorrelation(context.Background()); c.TraceID != "" {
		t.Error("expected empty correlation with no span")
	}
	tp := sdktrace.NewTracerProvider()
	ctx, span := tp.Tracer("t").Start(context.Background(), "op")
	defer span.End()
	c := ReadOtelCorrelation(ctx)
	if len(c.TraceID) != 32 || len(c.SpanID) != 16 {
		t.Errorf("trace/span id wrong: %+v", c)
	}
}

func TestAuthRoundTripperInjectsAndRetries(t *testing.T) {
	var tokenMints int32
	tokSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt32(&tokenMints, 1)
		// First token "stale", second "fresh".
		if n == 1 {
			_, _ = w.Write([]byte(`{"access_token":"stale","expires_in":3600}`))
		} else {
			_, _ = w.Write([]byte(`{"access_token":"fresh","expires_in":3600}`))
		}
	}))
	defer tokSrv.Close()

	var authSeen []string
	exportSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		authSeen = append(authSeen, r.Header.Get("Authorization"))
		if r.Header.Get("Authorization") == "Bearer stale" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer exportSrv.Close()

	tp, _ := NewTokenProvider(TokenProviderOptions{AuthURL: tokSrv.URL, ClientID: "c", ClientSecret: "s"})
	client := buildHTTPClient(SetupOtelOptions{TokenProvider: tp})
	if client == nil {
		t.Fatal("expected custom http client")
	}
	req, _ := http.NewRequest(http.MethodPost, exportSrv.URL, http.NoBody)
	resp, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Errorf("final status = %d", resp.StatusCode)
	}
	if len(authSeen) != 2 || authSeen[0] != "Bearer stale" || authSeen[1] != "Bearer fresh" {
		t.Errorf("auth retry sequence wrong: %v", authSeen)
	}
}

func TestSetupOtelSDKIdempotent(t *testing.T) {
	resetOtelSDK()
	defer resetOtelSDK()
	h1 := SetupOtelSDK(context.Background(), SetupOtelOptions{ServiceName: "svc", SkipStart: true})
	h2 := SetupOtelSDK(context.Background(), SetupOtelOptions{ServiceName: "other", SkipStart: true})
	if h1 != h2 {
		t.Error("SetupOtelSDK not idempotent")
	}
}

func TestSetupOtelSDKNoEndpointNoProviders(t *testing.T) {
	resetOtelSDK()
	defer resetOtelSDK()
	h := SetupOtelSDK(context.Background(), SetupOtelOptions{ServiceName: "svc", SkipStart: true})
	if h.TracerProvider != nil || h.MeterProvider != nil {
		t.Error("no endpoints should yield nil providers")
	}
}

// --- metrics integration: verify a counter records via a manual reader ---

func TestMetricsRecordsThroughOtel(t *testing.T) {
	resetMetricsInstrumentCache()
	reader := sdkmetric.NewManualReader()
	mp := sdkmetric.NewMeterProvider(sdkmetric.WithReader(reader))
	otel.SetMeterProvider(mp)
	defer otel.SetMeterProvider(otel.GetMeterProvider())

	m := GetMetricsClient("test-meter")
	m.Counter("widgets", 3, map[string]string{"color": "red"})
	stop := m.StartTimer("op.ms", nil)
	stop()

	var rm metricdata.ResourceMetrics
	if err := reader.Collect(context.Background(), &rm); err != nil {
		t.Fatal(err)
	}
	found := false
	for _, sm := range rm.ScopeMetrics {
		for _, mt := range sm.Metrics {
			if mt.Name == "widgets" {
				found = true
			}
		}
	}
	if !found {
		t.Error("counter not recorded through OTel meter provider")
	}
}

func TestMetricsWithTimingReturnsError(t *testing.T) {
	resetMetricsInstrumentCache()
	m := GetMetricsClient("test-meter")
	wantErr := context.DeadlineExceeded
	got := m.WithTiming(context.Background(), "op", func() error { return wantErr }, nil)
	if got != wantErr {
		t.Errorf("WithTiming did not propagate error: %v", got)
	}
}

// --- otel capture handler records on span ---

func TestOtelCaptureRecordsOnActiveSpan(t *testing.T) {
	resetOtelSDK()
	defer resetOtelSDK()
	sr := tracetest.NewSpanRecorder()
	tp := sdktrace.NewTracerProvider(sdktrace.WithSpanProcessor(sr))
	otel.SetTracerProvider(tp)
	defer otel.SetTracerProvider(otel.GetTracerProvider())

	c := NewClient()
	c.Init(ClientOptions{DSN: "x", Environment: "test"})
	RegisterOtelCapture(c, "test-tracer")

	ctx, span := tp.Tracer("h").Start(context.Background(), "request")
	c.CaptureExceptionOnSpan(ctx, context.Canceled, map[string]string{"k": "v"})
	span.End()

	ended := sr.Ended()
	if len(ended) == 0 {
		t.Fatal("no spans ended")
	}
	var reqSpan sdktrace.ReadOnlySpan
	for _, s := range ended {
		if s.Name() == "request" {
			reqSpan = s
		}
	}
	if reqSpan == nil {
		t.Fatal("request span not found")
	}
	if reqSpan.Status().Code.String() != "Error" {
		t.Errorf("span status = %s, want Error", reqSpan.Status().Code)
	}
	if len(reqSpan.Events()) == 0 {
		t.Error("expected recorded exception event on span")
	}
}

func TestGenAIAttributesNilSpanSafe(t *testing.T) {
	// Must not panic with a nil span.
	SetGenAIAttributes(nil, GenAIAttributes{System: "openai"})
	RecordGenAIMessage(nil, GenAIRoleUser, "hi", nil)
}

func TestGenAIAttributesSet(t *testing.T) {
	sr := tracetest.NewSpanRecorder()
	tp := sdktrace.NewTracerProvider(sdktrace.WithSpanProcessor(sr))
	_, span := tp.Tracer("g").Start(context.Background(), "llm")
	temp := 0.7
	in := int64(100)
	SetGenAIAttributes(span, GenAIAttributes{
		System:           "anthropic",
		OperationName:    GenAIOpChat,
		RequestModel:     "claude-opus-4-8",
		Temperature:      &temp,
		UsageInputTokens: &in,
		ToolNames:        []string{"search"},
	})
	RecordGenAIMessage(span, GenAIRoleAssistant, "hello", &GenAIMessageExtra{ToolName: "search"})
	span.End()

	s := sr.Ended()[0]
	attrs := map[string]any{}
	for _, kv := range s.Attributes() {
		attrs[string(kv.Key)] = kv.Value.AsInterface()
	}
	if attrs["gen_ai.system"] != "anthropic" {
		t.Errorf("system attr missing: %v", attrs)
	}
	if attrs["gen_ai.request.model"] != "claude-opus-4-8" {
		t.Errorf("model attr missing: %v", attrs)
	}
	if len(s.Events()) == 0 {
		t.Error("expected gen_ai message event")
	}
}

var _ = trace.SpanFromContext // keep trace import used if assertions change
var _ = time.Second
