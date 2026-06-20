package observability

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"testing"
)

func TestCaptureExceptionUninitialized(t *testing.T) {
	c := NewClient()
	if id := c.CaptureException(context.Background(), errors.New("x"), nil); id != "" {
		t.Errorf("uninitialized client returned id %q", id)
	}
}

func TestCaptureExceptionFiresTransportAndHandler(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "https://example/dsn", Environment: "test", Release: "r1"})

	var (
		mu          sync.Mutex
		transported []ObservabilityEvent
		handled     []ObservabilityEvent
	)
	c.RegisterTransport(func(batch []ObservabilityEvent) {
		mu.Lock()
		transported = append(transported, batch...)
		mu.Unlock()
	})
	c.RegisterCaptureHandler(func(e ObservabilityEvent, raw RawCapture) {
		mu.Lock()
		handled = append(handled, e)
		mu.Unlock()
		if raw.Err == nil {
			t.Error("handler raw.Err nil for exception")
		}
	})

	ctx := ContextWithScope(context.Background(), NewScope())
	SetUser(ctx, &User{ID: "u9"})
	id := c.CaptureException(ctx, errors.New("boom"), map[string]string{"k": "v"})

	if id == "" {
		t.Fatal("empty event id")
	}
	mu.Lock()
	defer mu.Unlock()
	if len(transported) != 1 || len(handled) != 1 {
		t.Fatalf("want 1 transported + 1 handled, got %d/%d", len(transported), len(handled))
	}
	e := transported[0]
	if e.Level != LevelError || e.Environment != "test" || e.Release != "r1" {
		t.Errorf("event fields wrong: %+v", e)
	}
	if len(e.Exception) != 1 || e.Exception[0].Value != "boom" {
		t.Errorf("exception wrong: %+v", e.Exception)
	}
	if e.User == nil || e.User.ID != "u9" {
		t.Errorf("user not applied from scope: %+v", e.User)
	}
	if e.Tags["k"] != "v" {
		t.Errorf("tags wrong: %+v", e.Tags)
	}
	if e.SDK.Name != sdkName || e.SDK.Runtime != RuntimeNode {
		t.Errorf("sdk info wrong: %+v", e.SDK)
	}
}

func TestCaptureMessageLevels(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "x"})
	var got ObservabilityEvent
	c.RegisterTransport(func(b []ObservabilityEvent) { got = b[0] })

	c.CaptureMessage(context.Background(), "hi", "")
	if got.Level != LevelInfo || got.Message != "hi" {
		t.Errorf("default level/message wrong: %+v", got)
	}
	c.CaptureMessage(context.Background(), "warn", LevelWarning)
	if got.Level != LevelWarning {
		t.Errorf("level not honored: %q", got.Level)
	}
}

func TestBeforeSendDrop(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "x", BeforeSend: func(e ObservabilityEvent) *ObservabilityEvent {
		return nil // drop everything
	}})
	called := false
	c.RegisterTransport(func(b []ObservabilityEvent) { called = true })
	id := c.CaptureException(context.Background(), errors.New("x"), nil)
	if called {
		t.Error("transport fired despite BeforeSend drop")
	}
	if id == "" {
		t.Error("dropped event should still return its id")
	}
}

func TestBeforeSendMutate(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "x", BeforeSend: func(e ObservabilityEvent) *ObservabilityEvent {
		if e.Tags == nil {
			e.Tags = map[string]string{}
		}
		e.Tags["added"] = "1"
		return &e
	}})
	var got ObservabilityEvent
	c.RegisterTransport(func(b []ObservabilityEvent) { got = b[0] })
	c.CaptureException(context.Background(), errors.New("x"), nil)
	if got.Tags["added"] != "1" {
		t.Errorf("BeforeSend mutation lost: %+v", got.Tags)
	}
}

func TestCaptureNeverPanics(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "x"})
	c.RegisterTransport(func(b []ObservabilityEvent) { panic("transport blew up") })
	c.RegisterCaptureHandler(func(e ObservabilityEvent, raw RawCapture) { panic("handler blew up") })
	// Must not panic.
	c.CaptureException(context.Background(), errors.New("x"), nil)
	c.CaptureMessage(context.Background(), "m", LevelInfo)
}

func TestToExceptionCauseChain(t *testing.T) {
	root := errors.New("root")
	wrapped := fmt.Errorf("outer: %w", root)
	exc := toException(wrapped)
	if exc.Value != "outer: root" {
		t.Errorf("outer value wrong: %q", exc.Value)
	}
	if exc.Cause == nil || exc.Cause.Value != "root" {
		t.Errorf("cause chain wrong: %+v", exc.Cause)
	}
	if len(exc.Stacktrace.Frames) == 0 {
		t.Error("expected captured frames")
	}
}

func TestToExceptionNil(t *testing.T) {
	exc := toException(nil)
	if exc.Type != "Unknown" {
		t.Errorf("nil error type = %q", exc.Type)
	}
	if exc.Stacktrace.Frames == nil {
		t.Error("frames should be non-nil empty slice for wire compat")
	}
}
