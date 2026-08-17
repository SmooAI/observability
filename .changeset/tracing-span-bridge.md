---
'@smooai/observability': minor
---

Rust: new `tracing-bridge` feature and `OtelSdkHandle::tracing_span_layer()`, so
`tracing` SPANS (`#[instrument]`, `info_span!`) actually export. The SDK set the
global tracer provider but never bridged `tracing` spans into it, so they were
printed and dropped while the service looked fully instrumented.
