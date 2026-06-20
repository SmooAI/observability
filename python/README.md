# `@smooai/observability` — Python

Python SDK for SmooAI Observability. Port of the TypeScript reference SDK
(`~/dev/smooai/observability/packages/core/src/`): error capture + breadcrumbs +
scoped context (webhook transport, SMOODEV-1148 dual-path), plus OpenTelemetry
traces + metrics export, GenAI semantic conventions, and an M2M token provider.

Tracking: [SMOODEV-1156](https://smooai.atlassian.net/browse/SMOODEV-1156).

## Install

```bash
pip install smooai-observability            # core: capture + webhook transport
pip install smooai-observability[otlp]      # + OTLP/HTTP trace & metric export
pip install smooai-observability[fastapi]   # + FastAPI/Starlette middleware
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

### Scoped context

```python
from smooai_observability import set_user, set_tag, add_breadcrumb, with_scope
from smooai_observability.types import User

set_user(User(id="u1", org_id="o1"))
add_breadcrumb("db", "query ran", {"rows": 12})

with with_scope() as scope:        # contextvars-based, async-safe
    scope.set_tag("request_id", "abc")
    ...                            # captures here pick up the child scope
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
    set_gen_ai_attributes(span, GenAIAttributes(
        system="anthropic", operation_name="chat",
        request_model="claude-opus-4-8",
        usage_input_tokens=120, usage_output_tokens=80,
    ))
```

### FastAPI

```python
from fastapi import FastAPI
from smooai_observability.integrations.fastapi import ObservabilityMiddleware

app = FastAPI()
app.add_middleware(ObservabilityMiddleware)   # after your auth middleware
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

## Development

```bash
uv sync --all-extras --dev
uv run ruff check . && uv run ruff format --check .
uv run pytest
```
