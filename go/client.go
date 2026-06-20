package observability

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"sync"

	"github.com/google/uuid"
)

// Client is the capture entry point — the Go analogue of the TS singleton
// _Client. It holds the configured options, an optional batched HTTP transport,
// and an optional runtime-native capture handler (used by the OTel integration
// to also record on spans). Both paths fire when both are registered, matching
// the TS SMOODEV-1148 behavior.
//
// A Client is safe for concurrent use. The package also exposes a process-wide
// default Client via Init / CaptureException / CaptureMessage for parity with
// the TS singleton ergonomics.
type Client struct {
	mu             sync.RWMutex
	options        *ClientOptions
	transport      func(batch []ObservabilityEvent)
	captureHandler CaptureHandler
}

// ClientOptions configures the Client. Mirrors the TS ClientOptions.
type ClientOptions struct {
	// DSN is the ingest endpoint: POST /webhooks/observability/{org_id}/{token}.
	DSN string
	// Environment is the deployment environment string.
	Environment string
	// Release is the release id (git sha or Lambda version).
	Release string
	// MaxQueueSize is the max events held in memory waiting to flush.
	MaxQueueSize int
	// FlushInterval is how often the transport flushes (default 1s).
	FlushInterval int // milliseconds
	// MaxBatchSize is the max events per flush batch (default 30).
	MaxBatchSize int
	// BeforeSend, if set, can mutate or drop an event (return nil to drop).
	BeforeSend func(ObservabilityEvent) *ObservabilityEvent
	// Runtime overrides the SDK runtime tag (default "node").
	Runtime Runtime
}

// CaptureHandler is a runtime-native capture path (e.g. OTel span events). When
// registered it fires IN ADDITION to the HTTP transport.
type CaptureHandler func(event ObservabilityEvent, raw RawCapture)

// RawCapture carries the original inputs alongside the prepared event so a
// handler can do runtime-specific work (e.g. span.RecordError(err)).
type RawCapture struct {
	Err     error
	Message string
	Tags    map[string]string
}

// NewClient returns an uninitialized client. Call Init before capturing.
func NewClient() *Client { return &Client{} }

// Init configures the client. Safe to call more than once (last wins).
func (c *Client) Init(opts ClientOptions) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if opts.Runtime == "" {
		opts.Runtime = RuntimeNode
	}
	c.options = &opts
}

// IsInitialized reports whether Init has been called.
func (c *Client) IsInitialized() bool {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.options != nil
}

// Options returns the current options (nil if uninitialized).
func (c *Client) Options() *ClientOptions {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.options
}

// RegisterTransport wires the batched-transport sink. Called by the HTTP
// transport setup.
func (c *Client) RegisterTransport(t func(batch []ObservabilityEvent)) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.transport = t
}

// RegisterCaptureHandler wires a runtime-native capture path. Pass nil to
// un-register.
func (c *Client) RegisterCaptureHandler(h CaptureHandler) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.captureHandler = h
}

// CaptureException records an error. ctx carries the scope (see WithScope /
// SetUser). Returns the event id, or "" if the client is uninitialized or the
// event was dropped before an id was assigned. Never panics.
func (c *Client) CaptureException(ctx context.Context, err error, tags map[string]string) (eventID string) {
	defer recoverSilently()

	c.mu.RLock()
	opts := c.options
	transport := c.transport
	handler := c.captureHandler
	c.mu.RUnlock()

	if opts == nil {
		return ""
	}
	eventID = uuid.NewString()

	event := ScopeFromContext(ctx).applyToEvent(ObservabilityEvent{
		EventID:     eventID,
		Timestamp:   nowMillis(),
		Level:       LevelError,
		Exception:   []ExceptionInfo{toException(err)},
		Tags:        tags,
		Release:     opts.Release,
		Environment: opts.Environment,
		SDK:         SDKInfo{Name: sdkName, Version: sdkVersion, Runtime: opts.Runtime},
	})

	final := applyBeforeSend(opts, event)
	if final == nil {
		return eventID
	}

	// Fire handler first so a throwing transport doesn't suppress OTel capture.
	if handler != nil {
		safeHandler(handler, *final, RawCapture{Err: err, Tags: tags})
	}
	if transport != nil {
		safeTransport(transport, []ObservabilityEvent{*final})
	}
	return eventID
}

// CaptureMessage records a one-line message at the given level (default
// LevelInfo). Returns the event id. Never panics.
func (c *Client) CaptureMessage(ctx context.Context, message string, level Level) (eventID string) {
	defer recoverSilently()

	c.mu.RLock()
	opts := c.options
	transport := c.transport
	handler := c.captureHandler
	c.mu.RUnlock()

	if opts == nil {
		return ""
	}
	if level == "" {
		level = LevelInfo
	}
	eventID = uuid.NewString()

	event := ScopeFromContext(ctx).applyToEvent(ObservabilityEvent{
		EventID:     eventID,
		Timestamp:   nowMillis(),
		Level:       level,
		Message:     message,
		Release:     opts.Release,
		Environment: opts.Environment,
		SDK:         SDKInfo{Name: sdkName, Version: sdkVersion, Runtime: opts.Runtime},
	})

	final := applyBeforeSend(opts, event)
	if final == nil {
		return eventID
	}

	if handler != nil {
		safeHandler(handler, *final, RawCapture{Message: message})
	}
	if transport != nil {
		safeTransport(transport, []ObservabilityEvent{*final})
	}
	return eventID
}

func applyBeforeSend(opts *ClientOptions, e ObservabilityEvent) *ObservabilityEvent {
	if opts.BeforeSend == nil {
		return &e
	}
	var out *ObservabilityEvent
	func() {
		defer recoverSilently()
		out = opts.BeforeSend(e)
	}()
	return out
}

func safeHandler(h CaptureHandler, e ObservabilityEvent, raw RawCapture) {
	defer recoverSilently()
	h(e, raw)
}

func safeTransport(t func([]ObservabilityEvent), batch []ObservabilityEvent) {
	defer recoverSilently()
	t(batch)
}

// toException converts an error (with its Unwrap chain) into ExceptionInfo,
// innermost frames first, walking errors.Unwrap for the cause chain like the TS
// SDK walks Error.cause.
func toException(err error) ExceptionInfo {
	if err == nil {
		return ExceptionInfo{Type: "Unknown", Value: "nil error", Stacktrace: Stacktrace{Frames: []StackFrame{}}}
	}
	exc := ExceptionInfo{
		Type:       errorTypeName(err),
		Value:      err.Error(),
		Stacktrace: Stacktrace{Frames: dropSdkFrames(captureStack(2))},
	}
	if cause := errors.Unwrap(err); cause != nil {
		c := toException(cause)
		exc.Cause = &c
	}
	return exc
}

// errorTypeName returns a stable type name for an error value, e.g.
// "*fmt.wrapError" or "errorString".
func errorTypeName(err error) string {
	t := reflect.TypeOf(err)
	if t == nil {
		return "Unknown"
	}
	if t.Kind() == reflect.Ptr {
		return "*" + t.Elem().String()
	}
	name := t.String()
	if name == "" {
		return fmt.Sprintf("%T", err)
	}
	return name
}

// recoverSilently swallows a panic so observability never crashes the host.
func recoverSilently() {
	_ = recover()
}

// --- process-wide default client (TS singleton parity) ---

// Default is the package-level client used by the top-level Init /
// CaptureException / CaptureMessage helpers.
var Default = NewClient()

// Init configures the default client.
func Init(opts ClientOptions) { Default.Init(opts) }

// CaptureException records an error on the default client.
func CaptureException(ctx context.Context, err error, tags map[string]string) string {
	return Default.CaptureException(ctx, err, tags)
}

// CaptureMessage records a message on the default client.
func CaptureMessage(ctx context.Context, message string, level Level) string {
	return Default.CaptureMessage(ctx, message, level)
}
