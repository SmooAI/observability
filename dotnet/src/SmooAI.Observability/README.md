# `SmooAI.Observability` — .NET

.NET SDK for SmooAI Observability — the .NET port of [`@smooai/observability`](https://github.com/SmooAI/observability).
Error capture, breadcrumbs, PII scrubbing, OpenTelemetry traces/metrics export,
GenAI semantic conventions, and an ASP.NET Core integration. Wire-compatible with
the TypeScript SDK so .NET and TS events land in the same Smoo dashboard with one
schema.

## Install

```xml
<PackageReference Include="SmooAI.Observability" Version="0.1.0" />
```

Targets `net8.0`, `net9.0`, and `net10.0`.

## Quick start

```csharp
using SmooAI.Observability;

// Env-driven, idempotent, never throws. Reads SMOOAI_OBSERVABILITY_* vars.
await Bootstrap.Run();

try
{
    DoWork();
}
catch (Exception ex)
{
    Sdk.Client.CaptureException(ex);
}
```

### Unhandled crashes

`Bootstrap.Run()` also subscribes process-level crash reporting, so a process that
dies from an unhandled exception — or silently drops a faulted `Task` nobody
awaited — reports instead of vanishing:

```csharp
// Wired automatically by Bootstrap.Run(). Call directly only if you build the
// OTel handle yourself. Idempotent.
GlobalHandlers.Register(otelHandle, flushTimeoutMs: 2000);
```

- Hooks `AppDomain.CurrentDomain.UnhandledException` and
  `TaskScheduler.UnobservedTaskException`. Both are multicast events, so handlers
  the host already attached still run and the runtime's own crash output is
  untouched.
- The crash becomes an ERROR-level event on the webhook transport **and** a
  semconv `exception` event on the current `Activity` (or a synthetic one) with
  the span status set to `Error`.
- `UnhandledException` cannot *prevent* termination on .NET Core — it is a
  notification. So the reporter force-flushes the exporters with a **bounded**
  2s budget before the runtime tears the process down; a wedged collector can't
  hold a dying process hostage.

### ASP.NET Core

```csharp
var app = builder.Build();
app.UseSmooObservability(); // capture unhandled exceptions + request breadcrumbs
```

### Scope / context

```csharp
Sdk.Client.SetUser(new UserContext { Id = "u1", OrgId = "o1" });
Sdk.Client.SetTag("feature", "checkout");
Sdk.Client.AddBreadcrumb("nav", "opened cart");

ObservabilityContext.WithScope(scope =>
{
    scope.SetTag("request", "abc");
    // captures here carry the forked scope
});
```

### Metrics

```csharp
using SmooAI.Observability.Metrics;

var metrics = MetricsClient.Get("my-service");
metrics.Counter("agent.turn.completed", 1, new Dictionary<string, string> { ["channel"] = "voice" });
metrics.Timing("agent.ttft.ms", 312);
using (metrics.StartTimer("tool.latency.ms")) { /* work */ }
```

### GenAI spans

```csharp
using System.Diagnostics;
using SmooAI.Observability.GenAI;

GenAIActivity.SetAttributes(Activity.Current, new GenAIAttributes
{
    System = "anthropic",
    OperationName = "chat",
    RequestModel = "claude-opus-4-8",
    UsageInputTokens = 1200,
    UsageOutputTokens = 340,
});
```

## Environment variables

| Variable | Purpose |
| --- | --- |
| `SMOOAI_OBSERVABILITY_ENDPOINT` | Base ingest URL (SDK appends `/v1/traces`, `/v1/metrics`) |
| `SMOOAI_OBSERVABILITY_TOKEN` | Pre-minted Bearer JWT (wins over client-credentials) |
| `SMOOAI_OBSERVABILITY_AUTH_URL` / `_CLIENT_ID` / `_CLIENT_SECRET` | M2M `client_credentials` auth |
| `SMOOAI_OBSERVABILITY_SERVICE_NAME` | OTel `service.name` (default `smoo-service`) |
| `SMOOAI_OBSERVABILITY_ENVIRONMENT` | Deployment environment |
| `SMOOAI_OBSERVABILITY_RELEASE` | Release id |
| `SMOOAI_OBSERVABILITY_DSN` | Error-webhook DSN (optional) |
| `SMOOAI_OBSERVABILITY_DISABLED` | `1`/`true` to skip bootstrap |
| `SMOOAI_OBSERVABILITY_PII_HASH_KEY` | HMAC key for hashing emails / phones / addresses (unset = redacted) |

## Design notes

- **Error-safe**: every public entry point swallows its own failures —
  observability never throws into the host.
- **Per-request auth**: OTLP export resolves a fresh Bearer from the
  `TokenProvider` on every request (via a delegating handler) so exports don't
  401 after the first token expires.
- **Wire format**: events serialize to the exact camelCase JSON shape of the TS
  SDK (`System.Text.Json`, nulls omitted).

Tracking: [SMOODEV-1159](https://smooai.atlassian.net/browse/SMOODEV-1159).

## PII scrubbing

Two classes, handled differently:

- **Credentials** (`Bearer …`, `password=`, `token`/`api_key`/`secret=`, `sk-…`)
  are **dropped**. A hash of a live token is still a token oracle.
- **Personal identifiers** (email, phone, street address) are **hashed**:
  `a@b.com` → `[email:9f2a41c8]`. The type prefix stays visible, so you can see
  *what kind* of value was there and that two spans carry the *same* one —
  without ever seeing it.

The hash is **HMAC-SHA256**, not a bare digest (emails and phones are a small
enumerable space a rainbow table reverses in seconds), and the org id is mixed
into the message so the same value hashes **differently in different orgs**.

Set the key with `SMOOAI_OBSERVABILITY_PII_HASH_KEY` (read by the bootstrap) or
`Pii.SetPiiHashKey(...)`. **With no key, personal identifiers are fully redacted**
(`[email:redacted]`) rather than hashed under a guessable one.

⚠️ **The key and the org id are load-bearing.** Rotating either silently breaks
correlation with every hash already stored — treat the key as permanent, and do
not reuse a secret that rotates on a schedule.

All five SDKs (TypeScript, Rust, Go, Python, .NET) emit byte-identical tokens
for the same key/org/value; the shared vectors are asserted in each SDK's PII
test suite.
