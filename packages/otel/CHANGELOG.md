# @smooai/observability-otel

## 0.3.0

### Minor Changes

- 365b90c: OTel-first node Client (SMOODEV-1067d).

    The Node Client no longer wraps a Smoo-native HTTP transport — it emits to OpenTelemetry natively. Every `captureException` / `captureMessage` becomes a span event on the active OTel span (or a synthetic one if none is active), with `SpanStatusCode.ERROR` for exceptions and OTLP-shaped attributes (`enduser.id`, `enduser.org_id`, `service.version`, `deployment.environment.name`, `smoo.tag.*`, `smoo.event_id`, `smoo.level`). The OTel SDK handles batching, retry, and wire format; the Smoo SDK does not run a parallel HTTP pipeline on Node.

    `@smooai/logger` is now optional. The Smoo SDK has no compile-time dependency on it. When present, its CONTEXT global feeds OTel baggage (see `@smooai/observability-otel`). When absent, the OTel ambient context (W3C trace context propagation, baggage) is the single source of correlation truth — winston / pino / bunyan / console users get the same trace-id flowing through logs, traces, and Smoo error groups by reading `readOtelCorrelation()`.

    Breaking changes (`@smooai/observability` 0.3 → 0.4):
    - `makeNodeTransport` (re-exported from the `node` entry) removed — no longer needed; OTel SDK is the transport.
    - `Client._registerTransport` is now a no-op on Node when a capture handler is registered (which happens by default in `Client.init`). Browser is unchanged.
    - New seam `Client._registerCaptureHandler(handler | null)` for advanced consumers who want to plug in their own non-OTel capture path.

    Breaking changes (`@smooai/observability-otel` 0.1 → 0.2):
    - `bridgeClientToOtel()` removed. There's nothing to bridge — the Smoo Client already emits to OTel natively on Node. `setupOtelSdk()` and `readOtelCorrelation()` remain.

    Tests: 33 green on core (was 24), 5 on otel package. Typecheck + build clean.

### Patch Changes

- Updated dependencies [365b90c]
    - @smooai/observability@0.4.0

## 0.2.0

### Minor Changes

- 2d2eed7: `@smooai/observability-otel` — OpenTelemetry foundation (SMOODEV-1067c Phase 1).

    New package wraps `@opentelemetry/sdk-node` + `@opentelemetry/auto-instrumentations-node` + the OTLP/HTTP trace exporter, and bridges the core `Client` so every `captureException` records on the active OTel span with `SpanStatusCode.ERROR`. Works without `@smooai/logger` — pipes correlation IDs through `@opentelemetry/api`'s ambient context, so any logger / framework that integrates with OTel sees the same trace-id flowing through logs, traces, and Smoo error groups.

    Public surface:
    - `setupOtelSdk({ serviceName, otlpEndpoint, otlpHeaders, environment, release, instrumentationConfig })` — idempotent Lambda / Node bootstrap. Returns `{ sdk, flush, shutdown }`.
    - `bridgeClientToOtel()` — wraps `Client.captureException` / `setUser` / `setTag` to also update OTel span attributes + status. Idempotent.
    - `readOtelCorrelation()` — read-only view of the active span's `traceId` / `spanId` / sampled flag.

    Also patches `@smooai/observability` core docs reference; no API change.

    12 tests (bridge + setup), typecheck + build clean.

### Patch Changes

- Updated dependencies [bd64532]
- Updated dependencies [2d2eed7]
    - @smooai/observability@0.3.0
