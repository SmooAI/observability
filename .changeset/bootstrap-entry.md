---
'@smooai/observability': minor
---

Add `@smooai/observability/bootstrap` subpath — a single side-effect import that customers (and Smoo internal services) use to instrument any Node compute (Lambda, ECS, Next.js Node runtime) without writing SDK glue.

```ts
// At the top of the entry file
import '@smooai/observability/bootstrap';
```

Then set env vars:

- `SMOOAI_OBSERVABILITY_ENDPOINT` — base URL of the ingest API (e.g. `https://api.smoo.ai`). SDK appends `/v1/traces` and `/v1/metrics`. Per-signal `OTEL_EXPORTER_OTLP_*_ENDPOINT` env vars are honored if set.
- Auth — pick ONE:
    - `SMOOAI_OBSERVABILITY_TOKEN` — pre-minted Bearer JWT. Easiest for local dev. Not refreshed.
    - `SMOOAI_OBSERVABILITY_AUTH_URL` + `SMOOAI_OBSERVABILITY_CLIENT_ID` + `SMOOAI_OBSERVABILITY_CLIENT_SECRET` — standard `client_credentials` flow. SDK posts to `${AUTH_URL}/token`, caches the JWT, re-mints every ~55min (under the openauth 1h TTL). The OTLP exporter reads the auth header by reference so refreshes propagate to the next export with no exporter restart.
- Optional: `SMOOAI_OBSERVABILITY_SERVICE_NAME`, `_ENVIRONMENT`, `_RELEASE`, `_DISABLED`.

Idempotent and crash-safe — calling `bootstrapObservability()` twice returns the same handle; missing config / mint failures / OTel init errors are logged to stderr without throwing.
