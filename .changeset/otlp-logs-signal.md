---
'@smooai/observability': minor
---

Add the OTLP logs signal. The SDK now wires a `LoggerProvider` +
`BatchLogRecordProcessor` alongside traces and metrics — same endpoint
(`SMOOAI_OBSERVABILITY_ENDPOINT` → `/v1/logs`), same auth (static token or
M2M client_credentials via the per-request `AuthInjectingLogExporter`), same
enable path. App logs emitted through the standard `@opentelemetry/api-logs`
facade become OTLP log records correlated to the active span's trace_id /
span_id; when no logs endpoint resolves the global LoggerProvider stays the
api-logs no-op and stdout output is unchanged.
