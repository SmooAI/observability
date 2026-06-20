# `@smooai/observability` — Go

Go SDK for SmooAI Observability, at feature parity with the TypeScript reference
SDK (`@smooai/observability/core`). Error/message capture with a Sentry-shaped
event envelope, a `context.Context`-carried scope, PII scrubbing, a batched HTTP
transport, OTLP trace + metric export with M2M token auth, a metrics helper,
GenAI semantic conventions, env-driven bootstrap, and a `net/http` middleware.

Canonical TS reference: `~/dev/smooai/observability/packages/core/src/`.
Tracking: [SMOODEV-1157](https://smooai.atlassian.net/browse/SMOODEV-1157).

**Observability must never panic the host.** Every public entry point recovers
from panics and swallows internal errors — the worst case is a dropped event.

## Quick start

```go
import obs "github.com/SmooAI/observability/go"

func main() {
    ctx := context.Background()

    // Env-driven bootstrap: reads SMOOAI_OBSERVABILITY_* vars, mints an M2M
    // token, wires OTel export + the Client + the OTel-native capture path.
    res := obs.Bootstrap(ctx, nil)
    if res.Otel != nil {
        defer res.Otel.Shutdown(ctx)
    }

    // Capture against a request/operation context (carries the scope + span).
    if err := doWork(); err != nil {
        obs.CaptureException(ctx, err, map[string]string{"area": "startup"})
    }
}
```

### Manual init (no bootstrap)

```go
obs.Init(obs.ClientOptions{
    DSN:         os.Getenv("OBSERVABILITY_DSN"),
    Environment: "production",
    Release:     gitSHA,
})
```

## Scope & context

The scope rides on `context.Context` (request-safe in concurrent servers — no
ambient-stack caveat):

```go
obs.SetUser(ctx, &obs.User{ID: "u1", OrgID: "org1"})
obs.SetTag(ctx, "feature", "checkout")
obs.AddBreadcrumb(ctx, "fetch", "GET /cart", nil, obs.LevelInfo)

obs.WithScope(ctx, func(ctx context.Context, scope *obs.Scope) {
    scope.SetTag("tx", "abc")          // isolated to this scope
    obs.CaptureException(ctx, err, nil) // sees the tx tag
})
```

Breadcrumb buffer is capped at 100 (oldest dropped), matching the TS SDK.

## OTel

```go
h := obs.SetupOtelSDK(ctx, obs.SetupOtelOptions{
    ServiceName:     "smooai-voice",
    Environment:     "production",
    TracesEndpoint:  "https://api.smoo.ai/v1/traces",
    MetricsEndpoint: "https://api.smoo.ai/v1/metrics",
    TokenProvider:   tp, // injects a fresh Bearer per export request
})
defer h.Shutdown(ctx)
```

Per-request auth (the SMOODEV-1206 fix) is implemented as a custom
`http.RoundTripper` handed to the stock OTLP HTTP exporters: a fresh token on
every export, retry-once on 401 — no header snapshot, no expiry drift.

## Metrics & GenAI

```go
m := obs.GetMetricsClient("smooai-voice")
m.Counter("agent.turn.completed", 1, map[string]string{"channel": "voice"})
stop := m.StartTimer("agent.tool.latency.ms", map[string]string{"tool": "search"})
defer stop()

obs.SetGenAIAttributes(span, obs.GenAIAttributes{
    System: "anthropic", OperationName: obs.GenAIOpChat,
    RequestModel: "claude-opus-4-8",
})
```

## net/http middleware

```go
mw := obs.NewMiddleware(obs.Default, func(r *http.Request) *obs.User {
    return &obs.User{ID: r.Header.Get("X-User-Id")}
})
http.Handle("/", mw(myHandler))
```

Establishes a request-scoped scope, records request context, and captures
downstream panics (then re-panics so the host's recovery still runs; set
`SwallowPanics: true` to write a 500 instead).

## Wire format

`ObservabilityEvent` JSON is byte-compatible with the TS `ObservabilityEvent`
so one backend ingest endpoint (`type: "error"`) serves both SDKs. The SDK name
(`@smooai/observability-go`) disambiguates the language; runtime is reported as
`node` (server-side).

## Gaps / deferred

- **Fiber / Gin / Echo adapters** — only `net/http` ships. They can be thin
  adapters over the same `Scope` + `CaptureException` primitives.
- **Span-implicit capture** — Go has no ambient span, so span-correlated
  capture uses `CaptureExceptionOnSpan(ctx, ...)` (reads the span off `ctx`).
  The plain `CaptureException` still records via transport + a synthetic span.
- **`sendBeacon` / page-unload** — browser-only; N/A for Go.
- **Source-map / symbolication** — Go stacks are already symbolicated.
