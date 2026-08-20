# `@smooai/observability` — Python

Python SDK for SmooAI Observability. Port of the TypeScript reference SDK
(`~/dev/smooai/observability/packages/core/src/`): error capture + breadcrumbs +
scoped context (webhook transport, SMOODEV-1148 dual-path), plus OpenTelemetry
traces + metrics export, GenAI semantic conventions, and an M2M token provider.

Tracking: [SMOODEV-1156](https://smooai.atlassian.net/browse/SMOODEV-1156).

## Install

> **Not yet published to PyPI** — publishing is tag-triggered (`python-v<semver>`
> in [`publish.yml`](../.github/workflows/publish.yml)) and no tag has shipped
> yet. Until then, install from source:

```bash
pip install "smooai-observability @ git+https://github.com/SmooAI/observability.git#subdirectory=python"
# extras once published: [otlp] OTLP/HTTP trace & metric export, [fastapi] FastAPI/Starlette middleware
```

## Quick start

```python
from smooai_observability import bootstrap_observability, capture_exception

bootstrap_observability()  # reads SMOOAI_OBSERVABILITY_* env vars (never raises)

try:
    risky()
except Exception as err:
    capture_exception(err, tags={"area": "ingest"})
```

### Crash reporting

`bootstrap_observability()` installs `sys.excepthook` **and** `threading.excepthook`, so a
process that dies from an uncaught exception (or a worker thread that dies from one) reports
before it goes away instead of vanishing silently — a CRITICAL log record plus a semconv
`exception` span event with the span status set to `ERROR`, force-flushed under a 2s budget.

It chains to whatever hook was already installed, so the interpreter's traceback still reaches
stderr, and it is idempotent. Install it by hand if you don't use `bootstrap_observability`:

```python
from smooai_observability import install_crash_handler

install_crash_handler()
```

Not covered: asyncio "Task exception was never retrieved". Those don't kill the process, and
asyncio already logs them at ERROR through the stdlib `asyncio` logger — which the root OTel
logging handler exports. An exception that escapes `asyncio.run` is a crash and is covered.

### Scoped context

```python
from smooai_observability import set_user, set_tag, add_breadcrumb, with_scope
from smooai_observability.types import User

set_user(User(id="u1", org_id="o1"))
add_breadcrumb("db", "query ran", {"rows": 12})

with with_scope() as scope:  # contextvars-based, async-safe
    scope.set_tag("request_id", "abc")
    ...  # captures here pick up the child scope
```

### Metrics

```python
from smooai_observability.otel import setup_otel_sdk
from smooai_observability.metrics import get_metrics_client

setup_otel_sdk(service_name="smooai-voice")
m = get_metrics_client("smooai-voice")
m.counter("agent.turn.completed", 1, {"channel": "voice"})
m.timing("agent.ttft.ms", 312, {"model": "sonnet"})
with m.with_timing("agent.tool.latency.ms", {"tool": "search"}):
    do_work()
```

### GenAI spans

```python
from opentelemetry import trace
from smooai_observability.gen_ai_attributes import GenAIAttributes, set_gen_ai_attributes

with trace.get_tracer("agent").start_as_current_span("llm.call") as span:
    set_gen_ai_attributes(
        span,
        GenAIAttributes(
            system="anthropic",
            operation_name="chat",
            request_model="claude-opus-4-8",
            usage_input_tokens=120,
            usage_output_tokens=80,
        ),
    )
```

### FastAPI

```python
from fastapi import FastAPI
from smooai_observability.integrations.fastapi import ObservabilityMiddleware

app = FastAPI()
app.add_middleware(ObservabilityMiddleware)  # after your auth middleware
```

## Environment variables

Same names as the TS bootstrap:

| Var | Purpose |
| --- | --- |
| `SMOOAI_OBSERVABILITY_ENDPOINT` | Base ingest URL; `/v1/traces` + `/v1/metrics` appended |
| `SMOOAI_OBSERVABILITY_TOKEN` | Pre-minted Bearer JWT (not refreshed) |
| `SMOOAI_OBSERVABILITY_AUTH_URL` / `_CLIENT_ID` / `_CLIENT_SECRET` | M2M `client_credentials` auth |
| `SMOOAI_OBSERVABILITY_DSN` | Webhook DSN for the Errors dashboard |
| `SMOOAI_OBSERVABILITY_SERVICE_NAME` | OTel `service.name` (default `smoo-service`) |
| `SMOOAI_OBSERVABILITY_ENVIRONMENT` / `_RELEASE` | Deployment env / release id |
| `SMOOAI_OBSERVABILITY_DISABLED` | `1`/`true` to skip bootstrap |
| `SMOOAI_OBSERVABILITY_PII_HASH_KEY` | HMAC key for hashing emails / phones / addresses (unset = redacted) |

## Development

```bash
uv sync --all-extras --dev
uv run ruff check . && uv run ruff format --check .
uv run pytest
```

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

Set the key with `SMOOAI_OBSERVABILITY_PII_HASH_KEY` (read by the bootstrap) or
`pii.set_pii_hash_key(...)`. **With no key, personal identifiers are fully redacted**
(`[email:redacted]`) rather than hashed under a guessable one.

⚠️ **The key and the org id are load-bearing.** Rotating either silently breaks
correlation with every hash already stored — treat the key as permanent, and do
not reuse a secret that rotates on a schedule.

All five SDKs (TypeScript, Rust, Go, Python, .NET) emit byte-identical tokens
for the same key/org/value; the shared vectors are asserted in each SDK's PII
test suite.
