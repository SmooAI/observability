# @smooai/observability

## 0.3.0

### Minor Changes

- bd64532: Node SDK capture handlers + Hono middleware (SMOODEV-1067 follow-up th-bafeb7).

    `@smooai/observability/node` now ships real implementations:
    - `registerNodeGlobalHandlers({ flush, exitOnUncaught })` — attaches `uncaughtException` + `unhandledRejection` listeners that forward to `Client.captureException`, plus optional SIGTERM / SIGINT / `beforeExit` flushing so a Lambda container shutdown drains the in-memory queue. Idempotent.
    - `makeNodeTransport(options)` — Node-flavored `Transport` adapter (fetch + keepalive, no Beacon). Returns the underlying transport so callers (and the auto-init wiring) can hook the flush method into the lifecycle.
    - `observabilityMiddleware({ resolveUser, requestHeaderAllowlist })` — Hono-shaped middleware. Per request: hydrates the active `Scope` with the authenticated user (defaults to reading `c.get('auth')` produced by `@smooai/auth`), adds a `request` context with method/path and an allow-listed header subset, wraps the handler chain in `withScope` so any `captureException` fired from a downstream handler picks up that request's identity, and captures thrown errors before re-throwing so Hono's onError still gets to render the response.
    - `Client.init` on node now auto-wires the transport and global handlers (override with `autoInstrumentation: false`).

    Also fixed a latent bug in `withScope`: previously the scope was popped before any `await` inside the callback resolved, so request-scoped state was gone by the time async handlers ran. `withScope` now defers the pop until a returned thenable settles, while keeping the synchronous fast path unchanged.

    24 tests total (was 13). Build + typecheck clean.

### Patch Changes

- 2d2eed7: `@smooai/observability-otel` — OpenTelemetry foundation (SMOODEV-1067c Phase 1).

    New package wraps `@opentelemetry/sdk-node` + `@opentelemetry/auto-instrumentations-node` + the OTLP/HTTP trace exporter, and bridges the core `Client` so every `captureException` records on the active OTel span with `SpanStatusCode.ERROR`. Works without `@smooai/logger` — pipes correlation IDs through `@opentelemetry/api`'s ambient context, so any logger / framework that integrates with OTel sees the same trace-id flowing through logs, traces, and Smoo error groups.

    Public surface:
    - `setupOtelSdk({ serviceName, otlpEndpoint, otlpHeaders, environment, release, instrumentationConfig })` — idempotent Lambda / Node bootstrap. Returns `{ sdk, flush, shutdown }`.
    - `bridgeClientToOtel()` — wraps `Client.captureException` / `setUser` / `setTag` to also update OTel span attributes + status. Idempotent.
    - `readOtelCorrelation()` — read-only view of the active span's `traceId` / `spanId` / sampled flag.

    Also patches `@smooai/observability` core docs reference; no API change.

    12 tests (bridge + setup), typecheck + build clean.

## 0.2.0

### Minor Changes

- 40bbb38: Browser capture MVP. Wires up `window.onerror` + `unhandledrejection` global handlers, optional `console.error` tap, `fetch` + navigation breadcrumb wrappers, batched `fetch` transport with `navigator.sendBeacon` flush on `pagehide`/`visibilitychange`, PII scrubbing (Bearer tokens, password/token/api-key params, OpenAI-style `sk-...` keys, sensitive headers), and an engine-agnostic V8 + Spidermonkey stack parser. `Client.init` now auto-installs everything when called from the browser entry. SDK-internal frames are stripped from captured stacks. `Error.cause` chains are walked into the exception envelope.
- ebda331: Initial 0.1.0 release. Universal browser + Node core with React and Next.js wrappers. Capture handlers and transport land incrementally — track follow-ups in [SmooAI/smooai SMOODEV-1067](https://github.com/SmooAI/smooai).

## 0.1.0

### Minor Changes

- Initial release. Universal browser + Node SDK skeleton with `Client.init`, `captureException`, `captureMessage`, `Scope` / `withScope`, breadcrumbs, and full TypeScript types covering the Sentry-shaped event envelope. Capture handlers, transport, and stack parsers land incrementally — see follow-up issues in [SmooAI/smooai](https://github.com/SmooAI/smooai) under SMOODEV-1067.
