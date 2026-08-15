---
'@smooai/observability': minor
---

TypeScript, Go, Python and .NET bootstraps now report whether they are actually
EXPORTING, not just whether they ran, and warn loudly when no OTLP endpoint is
configured — the same fix already landed for Rust.

`installed` / `Installed` kept its old meaning (bootstrap ran) and now says so
honestly in its doc comment; the new `exporting` / `Exporting` answers the
question that actually matters: does telemetry have anywhere to go. In TS and
Python the no-endpoint case is worse than a no-op — the OTel exporters fall back
to `http://localhost:4318` and retry into the void forever.
