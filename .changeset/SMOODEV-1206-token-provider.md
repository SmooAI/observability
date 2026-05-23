---
'@smooai/observability': minor
---

SMOODEV-1206: per-request TokenProvider auth — matches `@smooai/config` pattern, fixes silent OTLP 401s after token expiry.

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
