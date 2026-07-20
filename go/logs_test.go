package observability

import (
	"context"
	"log/slog"
	"sync"
	"testing"

	logglobal "go.opentelemetry.io/otel/log/global"
	sdklog "go.opentelemetry.io/otel/sdk/log"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
)

// capturingExporter records exported log records for assertions.
type capturingExporter struct {
	mu      sync.Mutex
	records []sdklog.Record
}

func (e *capturingExporter) Export(_ context.Context, records []sdklog.Record) error {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.records = append(e.records, records...)
	return nil
}
func (e *capturingExporter) Shutdown(context.Context) error   { return nil }
func (e *capturingExporter) ForceFlush(context.Context) error { return nil }

func (e *capturingExporter) all() []sdklog.Record {
	e.mu.Lock()
	defer e.mu.Unlock()
	return append([]sdklog.Record(nil), e.records...)
}

// A log emitted through SlogHandler with an active span in ctx must carry that
// span's real W3C trace_id/span_id — the correlation contract for /v1/logs.
func TestSlogHandlerCorrelatesWithActiveSpan(t *testing.T) {
	exp := &capturingExporter{}
	lp := sdklog.NewLoggerProvider(sdklog.WithProcessor(sdklog.NewSimpleProcessor(exp)))
	prev := logglobal.GetLoggerProvider()
	logglobal.SetLoggerProvider(lp)
	defer logglobal.SetLoggerProvider(prev)

	tp := sdktrace.NewTracerProvider()
	ctx, span := tp.Tracer("t").Start(context.Background(), "op")
	wantTrace := span.SpanContext().TraceID()
	wantSpan := span.SpanContext().SpanID()

	slog.New(SlogHandler("test")).WarnContext(ctx, "hello", "k", "v")
	span.End()

	recs := exp.all()
	if len(recs) != 1 {
		t.Fatalf("want 1 record, got %d", len(recs))
	}
	r := recs[0]
	if r.TraceID() != wantTrace {
		t.Errorf("trace id = %s, want %s (active span not read)", r.TraceID(), wantTrace)
	}
	if r.SpanID() != wantSpan {
		t.Errorf("span id = %s, want %s", r.SpanID(), wantSpan)
	}
	if r.Body().AsString() != "hello" {
		t.Errorf("body = %q, want hello", r.Body().AsString())
	}
	if r.Severity() != 0 && r.Severity() < 1 {
		t.Errorf("severity not set from level: %v", r.Severity())
	}
}

// No active span → no fabricated ids (zero trace/span), record still emitted.
func TestSlogHandlerNoSpanNoCorrelation(t *testing.T) {
	exp := &capturingExporter{}
	lp := sdklog.NewLoggerProvider(sdklog.WithProcessor(sdklog.NewSimpleProcessor(exp)))
	prev := logglobal.GetLoggerProvider()
	logglobal.SetLoggerProvider(lp)
	defer logglobal.SetLoggerProvider(prev)

	slog.New(SlogHandler("test")).Info("no span here")

	recs := exp.all()
	if len(recs) != 1 {
		t.Fatalf("want 1 record, got %d", len(recs))
	}
	if recs[0].TraceID().IsValid() {
		t.Errorf("expected no trace id without active span, got %s", recs[0].TraceID())
	}
}

// Disabled logs signal (no endpoint) → nil provider, graceful no-op.
func TestBuildLoggerProviderNoEndpointIsNoOp(t *testing.T) {
	if lp := buildLoggerProvider(context.Background(), SetupOtelOptions{}, "", buildResource(SetupOtelOptions{})); lp != nil {
		t.Error("expected nil LoggerProvider with no endpoint")
	}
	lp := buildLoggerProvider(context.Background(), SetupOtelOptions{}, "https://api.smoo.ai/v1/logs", buildResource(SetupOtelOptions{ServiceName: "svc"}))
	if lp == nil {
		t.Error("expected non-nil LoggerProvider when endpoint set")
	} else {
		_ = lp.Shutdown(context.Background())
	}
}
