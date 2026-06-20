# SmooAI Observability — Rust

Rust workspace for the SmooAI Observability SDK.

## Crates

- **[`observability`](observability/)** (`smooai-observability`) — the SDK:
  error capture, PII scrubbing, batched webhook transport, OpenTelemetry traces
  + metrics, GenAI semantic-conventions, and M2M auth. At parity with the
  TypeScript [`@smooai/observability`](../packages/core) reference SDK so Rust
  services (api-prime, voice, temporal-worker) can self-emit telemetry to
  `api.smoo.ai`.

See the [crate README](observability/README.md) for usage.

## Status

✅ SDK implemented and tested ([SMOODEV-1158](https://smooai.atlassian.net/browse/SMOODEV-1158)).
`cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` all green.
