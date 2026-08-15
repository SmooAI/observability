---
'@smooai/observability': minor
---

Rust bootstrap now reports whether it is actually EXPORTING, not just whether it
ran, and warns loudly when no OTLP endpoint is configured. `installed: true` with
no endpoint used to read as success while nothing left the process.
