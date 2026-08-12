---
'@smooai/observability': minor
---

Rust: errors now reach the trace. `bootstrap()` registers an OTel-native capture
handler and a panic hook, so `capture_exception` / `capture_message` record on
the active span (or a synthetic one) as a semconv `exception` event with an Error
status — matching the Go SDK's `otel_capture.go`, which Rust had no equivalent
of. A panicking service previously reported nothing anywhere.
