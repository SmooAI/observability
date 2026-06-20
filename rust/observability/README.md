# smooai-observability

Rust SDK for **SmooAI Observability** — error capture, PII scrubbing, batched
webhook transport, OpenTelemetry traces + metrics, GenAI semantic-conventions,
and M2M auth. At parity with the TypeScript [`@smooai/observability`][ts] SDK so
Rust services (api-prime, voice, temporal-worker) can self-emit telemetry to
`api.smoo.ai` over the **exact same wire format**.

Observability must never take down the host: every public entry point is
error-safe and degrades to a no-op (plus one stderr line) rather than panicking.

## Features

| Capability                         | TS reference                       | Rust |
| ---------------------------------- | ---------------------------------- | ---- |
| Event types (camelCase wire shape) | `types.ts`                         | ✅   |
| `capture_exception` / `_message`   | `client.ts`                        | ✅   |
| Stack capture (`backtrace`)        | `stack-parser.ts` (string parse)   | ✅   |
| Scope / context (per-task)         | `scope.ts`                         | ✅   |
| Breadcrumb buffer (max 100)        | `scope.ts`                         | ✅   |
| PII scrubbing                      | `pii.ts`                           | ✅   |
| Batched webhook transport + retry  | `transport.ts`                     | ✅   |
| OTLP traces + metrics export       | `otel/setup-otel-sdk.ts`           | ✅   |
| Per-request M2M auth (no staleness)| `otel/auth-injecting-exporter.ts`  | ✅   |
| Metrics client (counter/timing/…)  | `metrics/index.ts`                 | ✅   |
| GenAI semconv attributes + events  | `gen-ai-attributes.ts`             | ✅   |
| `TokenProvider` (client_credentials)| `auth/token-provider.ts`          | ✅   |
| Env-driven bootstrap (idempotent)  | `bootstrap/index.ts`               | ✅   |

## Quick start

```rust
use smooai_observability as obs;

#[tokio::main]
async fn main() {
    // Reads SMOOAI_OBSERVABILITY_ENDPOINT / _AUTH_URL / _CLIENT_ID / _CLIENT_SECRET
    // / _SERVICE_NAME / _ENVIRONMENT / _RELEASE / _DISABLED from the environment.
    let result = obs::bootstrap().await;
    obs::set_global_client(result.client.clone());

    obs::set_tag("component", "ingest-worker");
    obs::capture_message("worker started", obs::Level::Info);

    let metrics = obs::metrics_client("smooai-voice");
    metrics.counter("agent.turn.completed", 1, &[("channel", "voice")]);
    metrics.timing("agent.ttft.ms", 312.0, &[("model", "sonnet")]);

    // On shutdown, flush traces/metrics + any queued error events.
    if let Some(otel) = &result.otel { otel.flush(); }
    result.client.flush().await;
}
```

See [`examples/service_bootstrap.rs`](examples/service_bootstrap.rs) for a full
walkthrough (scope, error capture with cause chains, `with_scope`, metrics).

## Auth modes

- **Pre-minted token** — set `SMOOAI_OBSERVABILITY_TOKEN`. Not refreshed.
- **M2M `client_credentials`** — set `_AUTH_URL` + `_CLIENT_ID` + `_CLIENT_SECRET`.
  The OTLP exporter consults the [`TokenProvider`] on **every** export and
  re-mints on 401, so a rotated token is picked up on the next export with no
  exporter restart (the Rust analogue of the TS SMOODEV-1206 fix).

## GenAI spans

```rust
use opentelemetry::{global, trace::{Tracer, TracerProvider}};
use smooai_observability::{set_gen_ai_attributes, GenAIAttributes, GenAISystem, GenAIOperationName};

let tracer = global::tracer_provider().tracer("smooai-voice");
let mut span = tracer.start("llm.chat");
set_gen_ai_attributes(&mut span, &GenAIAttributes {
    system: Some(GenAISystem::Anthropic),
    operation_name: Some(GenAIOperationName::Chat),
    request_model: Some("claude-opus-4-8".into()),
    usage_input_tokens: Some(1200),
    usage_output_tokens: Some(340),
    ..Default::default()
});
```

## Consuming from a downstream service

```toml
[dependencies]
smooai-observability = { path = "../observability/rust/observability" } # or git
```

Requires a Tokio runtime (the transport flush loop + OTLP batch processor are
spawned tasks). Wire format is OTLP/HTTP/JSON, identical to the TS SDK.

[ts]: https://github.com/SmooAI/observability/tree/main/packages/core
[`TokenProvider`]: https://docs.rs/smooai-observability
