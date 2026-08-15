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
| PII scrubbing (credentials)        | `pii.ts`                           | ✅   |
| PII **hashing** (email/phone/addr) | — (Rust only)                      | ✅   |
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

## PII scrubbing

Two classes, handled differently:

- **Credentials** (`Bearer …`, `password=`, `token`/`api_key`/`secret=`, `sk-…`)
  are **dropped**. A hash of a live token is still a token oracle.
- **Personal identifiers** (email, phone, street address) are **hashed**:
  `a@b.com` → `[email:9f2a41c8]`. The type prefix stays visible, so you can see
  *what kind* of value was there and that two spans carry the *same* one —
  without ever seeing it.

The hash is **HMAC-SHA256**, not a bare digest (emails and phones are a small
enumerable space a rainbow table reverses in seconds), and the org id is mixed
into the message so the same value hashes **differently in different orgs**.

```rust
use smooai_observability::pii::{scrub_string_for_org, pii_token, PiiKind};

let scrubbed = scrub_string_for_org("mail a@b.com", org_id);   // "mail [email:9f2a41c8]"
// Search: hash the typed term the same way and match the stored token.
let needle = pii_token(PiiKind::Email, "A@B.com", org_id);
```

Set the key with `SMOOAI_OBSERVABILITY_PII_HASH_KEY` (read by `bootstrap()`) or
`pii::set_pii_hash_key`. **With no key, personal identifiers are fully redacted**
(`[email:redacted]`) rather than hashed under a guessable one.

⚠️ **The key and the org id are load-bearing.** Rotating either silently breaks
correlation with every hash already stored — treat the key as permanent, and do
not reuse a secret that rotates on a schedule.

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
