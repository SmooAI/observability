// Package observability is the SmooAI Observability Go SDK.
//
// It mirrors the TypeScript reference SDK (@smooai/observability/core) feature
// for feature: error/message capture with a Sentry-shaped event envelope, a
// context.Context-carried Scope, PII scrubbing, a batched HTTP transport, OTLP
// trace + metric export with M2M token auth, a metrics helper, GenAI semantic
// conventions, env-driven bootstrap, and a net/http middleware.
//
// Design rule inherited from the TS SDK: observability MUST NOT panic the host
// application. Every public entry point recovers from panics and swallows
// internal errors; the worst case is a dropped event, never a crashed process.
//
// See SMOODEV-1157.
package observability

// Level is the severity of an event. Most captured exceptions are LevelError.
type Level string

const (
	LevelFatal   Level = "fatal"
	LevelError   Level = "error"
	LevelWarning Level = "warning"
	LevelInfo    Level = "info"
	LevelDebug   Level = "debug"
)

// Runtime identifies the SDK runtime. Go always reports "node" so the backend's
// existing two-runtime fingerprinting (browser|node) treats it as server-side;
// the SDK name ("@smooai/observability-go") disambiguates the language.
type Runtime string

const (
	RuntimeNode    Runtime = "node"
	RuntimeBrowser Runtime = "browser"
)

const (
	sdkName    = "@smooai/observability-go"
	sdkVersion = "0.1.0"
)

// User is the user/org/session context attached to an event. All fields
// optional. JSON shape matches the TS `ObservabilityEvent['user']`.
type User struct {
	ID        string `json:"id,omitempty"`
	OrgID     string `json:"orgId,omitempty"`
	SessionID string `json:"sessionId,omitempty"`
}

// StackFrame is one frame of a captured stack, matching the TS wire format.
type StackFrame struct {
	// Module is the file or package path of the frame.
	Module string `json:"module"`
	// Function is the function name, if known.
	Function string `json:"function,omitempty"`
	// Lineno is the source line number.
	Lineno int `json:"lineno,omitempty"`
	// Colno is the source column. Go runtime stacks have no columns, so this is
	// always omitted, but the field exists for wire compatibility.
	Colno int `json:"colno,omitempty"`
	// InApp is true when the frame is application code (not the Go standard
	// library, runtime, or this SDK).
	InApp bool `json:"inApp"`
}

// ExceptionInfo is one error in the exception chain (innermost first).
type ExceptionInfo struct {
	// Type is the error type name, e.g. "*fmt.wrapError" or "errorString".
	Type string `json:"type"`
	// Value is the error message.
	Value string `json:"value"`
	// Stacktrace holds the captured frames.
	Stacktrace Stacktrace `json:"stacktrace"`
	// Cause is the wrapped error (errors.Unwrap chain), if any.
	Cause *ExceptionInfo `json:"cause,omitempty"`
}

// Stacktrace wraps the frame slice to match the TS `{ frames: StackFrame[] }`
// shape exactly.
type Stacktrace struct {
	Frames []StackFrame `json:"frames"`
}

// Breadcrumb is a single event in the breadcrumb trail leading to an event.
type Breadcrumb struct {
	// Timestamp is ms since epoch.
	Timestamp int64 `json:"timestamp"`
	// Category is free-form: "fetch", "navigation", "console", "custom", etc.
	Category string `json:"category"`
	// Level is "info" for most, "warning"/"error" for failures.
	Level Level `json:"level"`
	// Message is a short human-readable summary.
	Message string `json:"message,omitempty"`
	// Data is free-form structured data.
	Data map[string]any `json:"data,omitempty"`
}

// RequestInfo is the HTTP request (or invocation) context for an event.
type RequestInfo struct {
	URL         string            `json:"url,omitempty"`
	Method      string            `json:"method,omitempty"`
	Headers     map[string]string `json:"headers,omitempty"`
	QueryString string            `json:"queryString,omitempty"`
}

// SDKInfo is the SDK self-identification block.
type SDKInfo struct {
	Name    string  `json:"name"`
	Version string  `json:"version"`
	Runtime Runtime `json:"runtime"`
}

// ObservabilityEvent is the wire envelope. Field names + omitempty are chosen to
// produce JSON byte-compatible with the TS `ObservabilityEvent` so one backend
// ingest endpoint serves both SDKs.
type ObservabilityEvent struct {
	// EventID is a client-assigned UUID v4.
	EventID string `json:"eventId"`
	// Timestamp is ms since epoch.
	Timestamp int64 `json:"timestamp"`
	// Level is the severity.
	Level Level `json:"level"`
	// Message is an optional one-line message (for CaptureMessage).
	Message string `json:"message,omitempty"`
	// Exception is the exception chain (innermost first).
	Exception []ExceptionInfo `json:"exception,omitempty"`
	// Breadcrumbs is the buffer leading up to this event.
	Breadcrumbs []Breadcrumb `json:"breadcrumbs,omitempty"`
	// User is the user context, if known.
	User *User `json:"user,omitempty"`
	// Request is the request/invocation context.
	Request *RequestInfo `json:"request,omitempty"`
	// Tags are free-form key/value pairs for dashboard filtering.
	Tags map[string]string `json:"tags,omitempty"`
	// Contexts are free-form structured contexts (browser, os, device, ...).
	Contexts map[string]map[string]any `json:"contexts,omitempty"`
	// Release is the release identifier (git sha, Lambda version, ...).
	Release string `json:"release,omitempty"`
	// Environment is the deployment environment.
	Environment string `json:"environment,omitempty"`
	// SDK is the SDK self-identification.
	SDK SDKInfo `json:"sdk"`
}

// IngestPayload is the transport envelope POSTed to the Smoo ingest endpoint.
// Mirrors the TS discriminated union with `type: "error"`.
type IngestPayload struct {
	Type   string               `json:"type"`
	Events []ObservabilityEvent `json:"events"`
}
