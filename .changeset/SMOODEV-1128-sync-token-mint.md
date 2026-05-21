---
'@smooai/observability': minor
---

SMOODEV-1128: Bootstrap awaits the initial token mint before constructing the OTel SDK.

The OTel `@opentelemetry/exporter-trace-otlp-http@0.55+` exporter snapshots its `headers` config at construction via `Object.assign` (mergeHeaders in otlp-http-configuration). The previous fire-and-forget mint left the exporter holding an empty header object permanently — every export went out without `Authorization` and 401'd at any Bearer-auth-gated ingest endpoint.

**Breaking change**: `bootstrapObservability()` now returns `Promise<BootstrapResult>` instead of `BootstrapResult`. The side-effect import (`import '@smooai/observability/bootstrap'`) is unchanged for callers — top-level `await` handles the initial mint before any importing module sees the SDK.
