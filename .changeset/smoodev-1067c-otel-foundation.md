---
'@smooai/observability-otel': minor
'@smooai/observability': patch
---

`@smooai/observability-otel` — OpenTelemetry foundation (SMOODEV-1067c Phase 1).

New package wraps `@opentelemetry/sdk-node` + `@opentelemetry/auto-instrumentations-node` + the OTLP/HTTP trace exporter, and bridges the core `Client` so every `captureException` records on the active OTel span with `SpanStatusCode.ERROR`. Works without `@smooai/logger` — pipes correlation IDs through `@opentelemetry/api`'s ambient context, so any logger / framework that integrates with OTel sees the same trace-id flowing through logs, traces, and Smoo error groups.

Public surface:

- `setupOtelSdk({ serviceName, otlpEndpoint, otlpHeaders, environment, release, instrumentationConfig })` — idempotent Lambda / Node bootstrap. Returns `{ sdk, flush, shutdown }`.
- `bridgeClientToOtel()` — wraps `Client.captureException` / `setUser` / `setTag` to also update OTel span attributes + status. Idempotent.
- `readOtelCorrelation()` — read-only view of the active span's `traceId` / `spanId` / sampled flag.

Also patches `@smooai/observability` core docs reference; no API change.

12 tests (bridge + setup), typecheck + build clean.
