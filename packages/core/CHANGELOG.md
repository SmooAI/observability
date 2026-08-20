# @smooai/observability

## 0.19.2

### Patch Changes

- bdd092f: Every event this SDK has ever sent reported `sdk.version: "0.1.0"`.

  `packages/core/src/client.ts` hard-coded `SDK_VERSION = '0.1.0'` while the published package walked from 0.1.0 to 0.19.0. Eighteen minor releases of events landed in the backend labelled with the version of the first one, so "which SDK version produced this event?" — the question the field exists to answer — has been unanswerable for the entire life of the package. The Rust, Python, Go and .NET ports all carried the same frozen constant.

  The constant is now derived, not typed. `scripts/sync-versions.mjs` treats `packages/core/package.json` as the single source of truth and writes it into all eleven version-bearing files across the five SDKs — manifests (`Cargo.toml`, `pyproject.toml`, `.csproj`), lockfiles (`Cargo.lock`, `uv.lock`), and the reported-version constants in each language.

  It runs in the changesets **`version`** lifecycle, not after publish:

  ```jsonc
  "version": "changeset version && node scripts/sync-versions.mjs"
  ```

  That ordering is the fix, not a detail. The changesets action commits the working tree after `version`, so the synced files land in the release commit and every tag carries the versions it claims. Syncing after `publish` — the pattern in the sibling repos — mutates manifests in a CI workspace that is never committed, which is why those repos need `cargo publish --allow-dirty` to paper over the dirt.

  A `--check` mode runs on **every** PR (`pr-checks.yml`, deliberately not path-filtered) and fails on any mismatch. A `TARGETS` row whose pattern matches zero times, or more than once, is a hard error too — a silently-skipped target is the exact failure this script exists to prevent.

  One version across languages is the org's existing convention, not a new invention: `@smooai/fetch` is 3.4.1 on npm, crates.io and PyPI alike. The four unreleased SDKs are therefore set to 0.19.0 rather than starting over at 0.1.0.

## 0.19.1

### Patch Changes

- b4a608f: Rust, Python, Go and .NET: close the two GenAI divergences the README's ledger recorded.

  The TypeScript SDK is unchanged — this is the other four catching up to it, recorded here because one npm version is the whole SDK family's changelog.

  **`gen_ai.tool.names` is a string array in Rust too.** Rust emitted `names.join(",")` where TypeScript, Python, Go and .NET all emitted a string array. Two consequences: a backend filtering spans by tool could not do it against a Rust service's spans at all, and a tool name containing a comma silently became two tools. Now `Value::Array(Array::String(…))`, matching the other four and the OTel spec's array-valued attribute.

  **Recorded GenAI message content is PII-scrubbed in all five.** `recordGenAIMessage` scrubbed content in TypeScript only; the Rust, Python, Go and .NET ports wrote the raw string onto the span event. Prompts and tool arguments are the single most PII-dense payload this SDK can touch — raw emails, phone numbers, addresses and pasted credentials routinely appear in them — so every port now routes content through its own `scrubString` before the event is added. That drops credentials and hashes personal identifiers per-org, exactly as the TS reference does.

  Each fix ships with a span-level test in its own language (the assertion needs a real exported span, not a string), so the ledger row is now backed by CI rather than by prose.

  Note the scrub uses the **org-less** entry point in every SDK, so hashes are salted with the empty org: there is no org id in hand at this call site. Same as TypeScript. If an org id ever reaches here, switch to the `ForOrg` variant.

## 0.19.0

### Minor Changes

- b4ba6c4: Rust: new `tracing-bridge` feature and `OtelSdkHandle::tracing_span_layer()`, so
  `tracing` SPANS (`#[instrument]`, `info_span!`) actually export. The SDK set the
  global tracer provider but never bridged `tracing` spans into it, so they were
  printed and dropped while the service looked fully instrumented.

## 0.18.0

### Minor Changes

- 0e6dcd0: TypeScript, Go, Python and .NET: hash PII instead of leaking it, matching the Rust SDK.

  All four SDKs scrubbed **credentials only** — `Bearer`, `password=`,
  `token`/`api_key`/`secret=`, `sk-…` — while their module docs claimed "PII
  scrubbing". Emails, phone numbers and street addresses passed through to the
  backend untouched. Rust fixed this in #82; this brings the other four to parity
  with byte-identical output.

  Personal identifiers are now **hashed, not dropped**: `a@b.com` →
  `[email:9f2a41c8]`. The type prefix stays visible, so "are these two spans the
  same person?" stays answerable while nothing reversible is stored. The hash is
  **HMAC-SHA256, keyed** — not a bare digest, which a rainbow table reverses in
  seconds for a space as small as email addresses — and the org id is mixed into
  the message, so identical PII hashes differently in different orgs. Phone
  numbers normalize to digits and emails to lowercase before hashing, so
  `(415) 555-0142` and `415-555-0142` correlate.

  Credentials are still **dropped**, never hashed, and are matched first: a hash of
  a live token is a token oracle, and PII inside a secret (`token=a@b.com`) goes
  with the secret. With no key configured (`SMOOAI_OBSERVABILITY_PII_HASH_KEY`, or
  the per-SDK setter), personal identifiers are fully redacted (`[email:redacted]`)
  rather than hashed under a guessable one — fail closed, never fail open.

  New API, same shape in every SDK. The org-less entry points keep working
  unchanged (they hash under the empty org salt):

  - TypeScript: `setPiiHashKey`, `piiToken`, `scrubStringForOrg`,
    `scrubHeadersForOrg`, `PiiKind` — now exported from the package entry
  - Go: `SetPiiHashKey`, `PiiToken`, `ScrubStringForOrg`, `ScrubHeadersForOrg`,
    `PiiKind`, `BootstrapEnv.PiiHashKey`
  - Python: `set_pii_hash_key`, `pii_token`, `scrub_string_for_org`,
    `scrub_headers_for_org`, `PiiKind`, `BootstrapEnv.pii_hash_key`
  - .NET: `Pii.SetPiiHashKey`, `Pii.PiiToken`, `Pii.ScrubStringForOrg`,
    `Pii.ScrubHeadersForOrg`, `PiiKind`, `BootstrapEnv.PiiHashKey`

  `piiToken(kind, raw, orgId)` is the search seam: hash a typed query term the same
  way and match the stored token.

  ⚠️ **The key is load-bearing — rotate never.** Rotating it silently forks
  correlation with every hash already stored. Supply it once at startup; the
  setters are set-once and refuse a second key.

  The TypeScript SDK ships a small synchronous SHA-256/HMAC (`hmac-sha256.ts`)
  rather than taking a dependency: `scrubString` is sync and runs in the browser
  bundle, where `node:crypto` is unavailable and WebCrypto is async-only. It is
  pinned by the RFC 4231 and FIPS 180-4 vectors.

## 0.17.0

### Minor Changes

- 7aecf45: TypeScript, Go, Python and .NET bootstraps now report whether they are actually
  EXPORTING, not just whether they ran, and warn loudly when no OTLP endpoint is
  configured — the same fix already landed for Rust.

  `installed` / `Installed` kept its old meaning (bootstrap ran) and now says so
  honestly in its doc comment; the new `exporting` / `Exporting` answers the
  question that actually matters: does telemetry have anywhere to go. In TS and
  Python the no-endpoint case is worse than a no-op — the OTel exporters fall back
  to `http://localhost:4318` and retry into the void forever.

## 0.16.0

### Minor Changes

- d40f623: Rust bootstrap now reports whether it is actually EXPORTING, not just whether it
  ran, and warns loudly when no OTLP endpoint is configured. `installed: true` with
  no endpoint used to read as success while nothing left the process.
- 1104624: Instrument the OpenAI Node SDK, and stop leaking prompt content into GenAI span events.

  `wrapOpenAI(client, options)` returns a proxy of an OpenAI client whose `chat.completions.create` emits OTel GenAI semantic-convention spans — request model / sampling params / tool names on the way out, response model, id, finish reason, and token usage on the way back. Streaming is handled: the span stays open until the stream drains (or the consumer breaks out early), and picks up the usage chunk emitted under `stream_options.include_usage`. The client is duck-typed, so this adds no dependency on `openai` and works against Groq, Together, Fireworks, DeepSeek, Azure OpenAI, and any OpenAI-compatible gateway via `{ system: 'groq' }`.

  Cost has a seam now: nothing in the platform computes an LLM price on its own, which is why the dashboard's cost column is empty. Pass `costUsd({ requestModel, responseModel, inputTokens, outputTokens, cachedTokens })` and it lands on `gen_ai.usage.cost_usd`.

  `recordGenAIMessage` now routes content through the SDK's PII scrub before it leaves the process. Prompts are the most PII-dense payload this SDK can touch, and `wrapOpenAI` keeps content recording **off** by default (`{ recordContent: true }` opts in).

## 0.15.0

### Minor Changes

- 5648be2: Rust: PII is now hashed rather than passed through. `pii::scrub_string` handled
  credentials only — `Bearer`, `password=`, `token`/`api_key`/`secret=`, `sk-…` —
  while the module doc claimed PII scrubbing, so an email or phone in a message,
  breadcrumb or GenAI tool argument reached the wire intact.

  Emails, phone numbers and street addresses are now detected and replaced with a
  keyed token: `a@b.com` → `[email:9f2a41c8]`. HMAC-SHA256, not a bare digest —
  those values are a small enumerable space a rainbow table reverses in seconds —
  and the org id is mixed into the message so identical PII hashes differently in
  different orgs. The type prefix stays visible, which keeps "are these two spans
  the same person?" answerable while storing nothing reversible.

  Credentials are still **dropped**, never hashed: a hash of a live token is a
  token oracle. With no key configured (`SMOOAI_OBSERVABILITY_PII_HASH_KEY`, or
  `pii::set_pii_hash_key`), personal identifiers are fully redacted rather than
  hashed under a guessable key.

  New: `pii::scrub_string_for_org`, `pii::scrub_headers_for_org`, `pii::pii_token`,
  `pii::PiiKind`, `pii::set_pii_hash_key`, `BootstrapEnv::pii_hash_key`.
  `scrub_string` / `scrub_headers` keep their signatures and now scrub personal
  identifiers too, under the empty org salt.

## 0.14.0

### Minor Changes

- eb02e38: Rust: errors now reach the trace. `bootstrap()` registers an OTel-native capture
  handler and a panic hook, so `capture_exception` / `capture_message` record on
  the active span (or a synthetic one) as a semconv `exception` event with an Error
  status — matching the Go SDK's `otel_capture.go`, which Rust had no equivalent
  of. A panicking service previously reported nothing anywhere.

## 0.13.0

### Minor Changes

- 654f271: Add the OTLP logs signal. The SDK now wires a `LoggerProvider` +
  `BatchLogRecordProcessor` alongside traces and metrics — same endpoint
  (`SMOOAI_OBSERVABILITY_ENDPOINT` → `/v1/logs`), same auth (static token or
  M2M client_credentials via the per-request `AuthInjectingLogExporter`), same
  enable path. App logs emitted through the standard `@opentelemetry/api-logs`
  facade become OTLP log records correlated to the active span's trace_id /
  span_id; when no logs endpoint resolves the global LoggerProvider stays the
  api-logs no-op and stdout output is unchanged.

## 0.12.0

### Minor Changes

- a071796: SMOODEV-2698 (ADR-097 W1+W2): session-scoped browser log sampling, config-served telemetry settings, and the cross-language parity corpus.
  - `sampleDecision(id, ratio)` — deterministic FNV-1a 32-bit over the UTF-8 bytes of the session/trace id, so the decision is stable for a page's lifetime and reproducible byte-identically in the Rust/Python/Go/.NET SDKs. Ratio 0.0/1.0 are exact.
  - `shouldEmitLog(...)` — one decision point: kill switch → minimum level → warnings/errors always 100% → trace decision inherited where a trace exists → otherwise the session decision. Sampling is per session, never per line, so any trace you can open has 100% of its log lines.
  - `loadTelemetrySettings(provider)` / `resolveTelemetrySettings(raw)` — `@smooai/config` public-tier telemetry settings read through an injectable provider seam (the SDK never imports a config client, so it stays usable with no network). Unreachable, malformed, or out-of-range values fall back to the compiled-in ADR-010 defaults, never to "sample everything out".
  - `parseTraceparent` / `formatTraceparent` — the first real W3C trace-context implementation in this SDK; strict, rejects all-zero ids.
  - `normalizeLevel` — canonical UPPERCASE levels, because ADR-096's error-rate query is case-sensitive.
  - `parity/sampling-corpus.json` — 170 committed golden vectors every language SDK asserts against in its own CI lane.

## 0.11.0

### Minor Changes

- 82bb589: SMOODEV-1206: per-request TokenProvider auth — matches `@smooai/config` pattern, fixes silent OTLP 401s after token expiry.

  The previous bootstrap minted a Bearer once at SDK init and stuck it in a
  headers map. The OTel JS v0.55 OTLP HTTP exporter `Object.assign`s that
  map at construction time, so the original snapshot lived forever — every
  export 401'd after the first token expired (~1h). Voice ECS containers
  running for hours past expiry lost every span; warm Lambdas inherited
  stale snapshots.

  Fix: new `TokenProvider` (direct port of `@smooai/config`'s) that caches
  a token in memory, refreshes 60s before expiry, dedupes concurrent
  calls, and exposes `invalidate()` for 401 retry. New custom
  `AuthInjectingTraceExporter` + `AuthInjectingMetricExporter` ask the
  TokenProvider for a fresh Bearer on EVERY export — no snapshot.

  `setupOtelSdk` now accepts a `tokenProvider` option; when set it routes
  traces + metrics through the new exporters. The static-token path
  (`SMOOAI_OBSERVABILITY_TOKEN`) and `otlpHeaders` snapshot path are
  preserved for callers that want to handle auth themselves.

## 0.10.1

### Patch Changes

- 170b137: fix(node): re-export `setGenAIAttributes` / `recordGenAIMessage` / GenAI types from node entry — were missing in 0.10.0, broke backend builds importing the helpers from the bare package name

## 0.10.0

### Minor Changes

- 59234b2: SMOODEV-1155 + SMOODEV-1156–1159: scaffold multi-language SDK subdirs and add OTel GenAI semantic-conventions helpers.
  - New scaffolds under `dotnet/`, `go/`, `python/`, `rust/` mirroring the layout of `~/dev/smooai/logger/`. Each is a placeholder package manifest + README pointing at the canonical TS reference and its tracking ticket.
  - New `setGenAIAttributes(span, attrs)` + `recordGenAIMessage(span, role, content)` helpers for emitting the OTel `gen_ai.*` attribute family on LLM and agent spans. Backs the upcoming LLM Observability dashboard (SMOODEV-1160).

## 0.9.0

### Minor Changes

- 7454d83: SMOODEV-1148: Node Client.captureException now fires BOTH OTel capture AND HTTP webhook transport.

  Previously the runtime-native captureHandler (OTel span events) short-circuited the HTTP transport, so Node errors never reached the webhook-backed Errors dashboard. Now both paths fire: OTel keeps emitting span events for tracing/observability, and the webhook also gets the event for the Errors UI.

  Node init now registers an HTTP transport (`makeNodeTransport`) when a `dsn` is configured. No-op when DSN is empty.

## 0.8.0

### Minor Changes

- a956c06: SMOODEV-1128: Bootstrap awaits the initial token mint before constructing the OTel SDK.

  The OTel `@opentelemetry/exporter-trace-otlp-http@0.55+` exporter snapshots its `headers` config at construction via `Object.assign` (mergeHeaders in otlp-http-configuration). The previous fire-and-forget mint left the exporter holding an empty header object permanently — every export went out without `Authorization` and 401'd at any Bearer-auth-gated ingest endpoint.

  **Breaking change**: `bootstrapObservability()` now returns `Promise<BootstrapResult>` instead of `BootstrapResult`. The side-effect import (`import '@smooai/observability/bootstrap'`) is unchanged for callers — top-level `await` handles the initial mint before any importing module sees the SDK.

## 0.7.0

### Minor Changes

- 3b91840: Add `@smooai/observability/bootstrap` subpath — a single side-effect import that customers (and Smoo internal services) use to instrument any Node compute (Lambda, ECS, Next.js Node runtime) without writing SDK glue.

  ```ts
  // At the top of the entry file
  import "@smooai/observability/bootstrap";
  ```

  Then set env vars:

  - `SMOOAI_OBSERVABILITY_ENDPOINT` — base URL of the ingest API (e.g. `https://api.smoo.ai`). SDK appends `/v1/traces` and `/v1/metrics`. Per-signal `OTEL_EXPORTER_OTLP_*_ENDPOINT` env vars are honored if set.
  - Auth — pick ONE:
    - `SMOOAI_OBSERVABILITY_TOKEN` — pre-minted Bearer JWT. Easiest for local dev. Not refreshed.
    - `SMOOAI_OBSERVABILITY_AUTH_URL` + `SMOOAI_OBSERVABILITY_CLIENT_ID` + `SMOOAI_OBSERVABILITY_CLIENT_SECRET` — standard `client_credentials` flow. SDK posts to `${AUTH_URL}/token`, caches the JWT, re-mints every ~55min (under the openauth 1h TTL). The OTLP exporter reads the auth header by reference so refreshes propagate to the next export with no exporter restart.
  - Optional: `SMOOAI_OBSERVABILITY_SERVICE_NAME`, `_ENVIRONMENT`, `_RELEASE`, `_DISABLED`.

  Idempotent and crash-safe — calling `bootstrapObservability()` twice returns the same handle; missing config / mint failures / OTel init errors are logged to stderr without throwing.

### Patch Changes

- 2984514: Drop unused `tsup` root devDependency — the package builds with tsdown.

## 0.4.0

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
