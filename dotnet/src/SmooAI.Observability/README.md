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

## Design notes

- **Error-safe**: every public entry point swallows its own failures —
  observability never throws into the host.
- **Per-request auth**: OTLP export resolves a fresh Bearer from the
  `TokenProvider` on every request (via a delegating handler) so exports don't
  401 after the first token expires.
- **Wire format**: events serialize to the exact camelCase JSON shape of the TS
  SDK (`System.Text.Json`, nulls omitted).

Tracking: [SMOODEV-1159](https://smooai.atlassian.net/browse/SMOODEV-1159).
