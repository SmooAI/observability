// Package observability is the SmooAI Observability Go SDK.
//
// Phase 1 of this package will mirror @smooai/observability/core:
//   - Client.CaptureException(err) — POSTs to the configured DSN webhook
//   - Init(opts) — wires the global OTel TracerProvider when present
//
// See SMOODEV-1157 for the implementation roadmap. The wire contract is
// documented in packages/core/src/types.ts (TypeScript) — the webhook
// accepts {type: "error", events: ObservabilityEvent[]}.
package observability

// Placeholder — implementation lands per SMOODEV-1157.
