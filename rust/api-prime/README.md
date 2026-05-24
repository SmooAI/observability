# smooai-api-prime

Rust api-prime crate for SmooAI. Renamed from `smooai-api-prime` per
**ADR-017** (Edge Mesh / api-prime split). Houses two binaries from a
single workspace member:

- `api-prime` — data plane. HPA-scaled, stateless. Today serves the
  high-traffic sidebar / auth endpoints that are too cold-start-prone
  on Lambda. Wave 3 turns it into a thin reverse-proxy fronting a
  manifest-driven route table.
- `api-prime-controller` — control plane. Single replica. Owns the
  route table state in Valkey, exposes an admin API + an internal
  cache-invalidate API consumed by data-plane pods.

This crate originated as the **SMOODEV-1227 scaffold** (Phase 5c of the
EKS migration plan). The rename + controller skeleton land under
**SMOODEV-1272**.

## Endpoints

| Method | Path                | Status  | Notes                                                           |
| ------ | ------------------- | ------- | --------------------------------------------------------------- |
| GET    | `/health/liveness`  | working | process alive                                                   |
| GET    | `/health/readiness` | working | pings Postgres + Redis                                          |
| GET    | `/v1/profile`       | working | port of `packages/backend/src/routes/profile.ts` (GET)          |
| POST   | `/v1/auth/sign-in`  | working | Supabase password-grant passthrough, rate-limited 10/60s per IP |
| POST   | `/v1/auth/refresh`  | working | Supabase refresh-token-grant passthrough                        |

`GET /v1/profile` requires `Authorization: Bearer <supabase-jwt>`.

### Auth endpoints

Both auth endpoints proxy to Supabase GoTrue (`{SUPABASE_URL}/auth/v1/token`)
and return the upstream JSON body **verbatim** on success — the dashboard
consumes `{access_token, refresh_token}` and hands them to
`@supabase/ssr`'s `setSession()` to write cookies in the standard
`sb-{project-ref}-auth-token` format.

Error mapping:

| Upstream              | Returned to caller                              |
| --------------------- | ----------------------------------------------- |
| 200 OK                | 200 with verbatim GoTrue body                   |
| 400/401 invalid_grant | 401 `{"message": "Invalid login credentials"}`  |
| 429                   | 429 `{"message": "Too many sign-in attempts."}` |
| 5xx / network error   | 502 `{"message": "Auth provider unavailable"}`  |

Rate limit on `/v1/auth/sign-in` is per-IP (`X-Forwarded-For` left-most,
falling back to the socket peer), 10 attempts per 60s, Redis-backed. The
limiter **fails open** on Redis outage — auth latency takes priority over
strict throttling. Rate-limit responses include `Retry-After` (seconds).

### Divergence from the TS implementation

The TS profile route calls `supabase.auth.getUser()` to fetch
`app_metadata.org_id`. The Rust impl **does not** — we rely on the
decoded JWT claims (`sub`, `email`, `aud`) only. If org context is needed
in a future endpoint, query `organization_members` by `user_id` via sqlx.
This eliminates a ~50ms Supabase HTTP roundtrip per request — the whole
point of having a hot-path service.

## Environment variables

| Var                 | Required | Default                                         | Description                                     |
| ------------------- | -------- | ----------------------------------------------- | ----------------------------------------------- |
| `DATABASE_URL`      | yes      | —                                               | Postgres connection string                      |
| `SUPABASE_URL`      | yes      | —                                               | e.g. `https://xrqbqgotghitcfuoukdk.supabase.co` |
| `SUPABASE_ANON_KEY` | yes      | —                                               | used by `/v1/auth/sign-in` + `/v1/auth/refresh` |
| `SUPABASE_JWKS_URL` | no       | `${SUPABASE_URL}/auth/v1/.well-known/jwks.json` | JWKS endpoint for JWT verification              |
| `REDIS_URL`         | no       | `redis://127.0.0.1:6379`                        | Redis for read-through cache                    |
| `PORT`              | no       | `3000`                                          | HTTP listen port                                |
| `RUST_LOG`          | no       | `info`                                          | tracing env filter                              |

## Local development

```bash
# From the repo root (observability/)
cd rust

# Build (uses the workspace at rust/Cargo.toml)
cargo build -p smooai-api-prime

# Release build
cargo build --release -p smooai-api-prime

# Run tests
cargo test -p smooai-api-prime

# Lint
cargo clippy -p smooai-api-prime --all-targets -- -D warnings

# Run it (requires a reachable Postgres + Supabase)
DATABASE_URL=postgres://postgres@127.0.0.1:54322/postgres \
SUPABASE_URL=https://xrqbqgotghitcfuoukdk.supabase.co \
SUPABASE_ANON_KEY=<anon-key> \
cargo run -p smooai-api-prime
```

The integration tests in `tests/integration_profile.rs` use
`sqlx::PgPool::connect_lazy`, so they pass without a live database — they
exercise the routing + auth-header parsing only.

## Docker

The Dockerfile lives at `rust/api-prime/Dockerfile` but its build context
must be the workspace root `rust/`, because cargo needs to see the
sibling members listed in `rust/Cargo.toml`.

```bash
# From observability repo root
docker buildx build \
    --platform linux/arm64 \
    -t ghcr.io/smooai/api-prime:dev \
    -f rust/api-prime/Dockerfile \
    rust/
```

Multi-stage: rust:1.83-slim-bookworm builder → distroless/cc-debian12
runtime, runs as the distroless `nonroot` user (`65532:65532`),
exposes port 3000.

## Architecture notes

- **JWKS cache**: 1-hour TTL. On a `kid` miss we force a single refresh
  before failing the request. See `src/auth/jwt.rs`.
- **JWT alg**: ES256 only. Supabase v2 projects default to ES256; HS256
  support would need the shared secret wired in via `@smooai/config` and
  is intentionally out of scope for this PR.
- **Postgres pool**: max 10 connections, 5s acquire timeout.
- **Cache key**: `profile:{user_id}` with TTL `300s`. Cache writes are
  best-effort — Redis failures never break a request.
- **Performance design target**: p99 < 20ms warm for `/v1/profile`. Not
  benched yet; will be measured by the Phase 6 shadow harness.

## Edge pipeline (SMOODEV-1276 + SMOODEV-1278)

The data-plane binary now runs a programmable edge pipeline per
ADR-017. Single catch-all axum route → `src/edge/dispatcher.rs`:

```
request → routes.lookup → auth.verify → ratelimit.check →
    schema.validate_request → dispatch(proxy | cache | implement)
```

Modules under `src/edge/`:

| Module        | Owns                                                                     |
| ------------- | ------------------------------------------------------------------------ |
| `route_table` | `apr:route:*` reader + RCU swap on `apr:config-bump`                     |
| `auth`        | JWT (user) + M2M token verification, produces `EdgeAuthContext`          |
| `ratelimit`   | Valkey sliding-window per `<sub>:<route_hash>`                           |
| `cache`       | moka L1 + Valkey L2 with stale-while-revalidate                          |
| `pubsub`      | subscriber for `apr:config-bump` + `apr:invalidate`                      |
| `proxy`       | direct Lambda invoke (no API Gateway hop, ADR-017 pivot)                 |
| `edge_attest` | HMAC-signed attestation embedded in `requestContext.authorizer.smooEdge` |
| `implement`   | static dispatch to in-process Rust handlers                              |
| `schema`      | v1 stub — real validator lands in SMOODEV-1277                           |
| `debug`       | dev-only response headers (`X-Smoo-Cache-Status`, etc.)                  |

### Required env vars (data plane)

| Name                    | Default       | Purpose                                                |
| ----------------------- | ------------- | ------------------------------------------------------ |
| `DATABASE_URL`          | (required)    | Postgres pool for implement-mode handlers              |
| `REDIS_URL`             | `redis://...` | Valkey for route table + cache + rate limit            |
| `SUPABASE_URL`          | (required)    | JWKS base URL                                          |
| `SUPABASE_ANON_KEY`     | (required)    | Auth-passthrough endpoints                             |
| `SUPABASE_JWKS_URL`     | derived       | Override JWKS endpoint                                 |
| `EDGE_ATTEST_SECRET`    | (required)    | HMAC secret for trust-boundary attestation             |
| `CACHE_L1_MAX_ENTRIES`  | `10000`       | moka capacity                                          |
| `SHUTDOWN_TIMEOUT_SECS` | `25`          | Graceful drain budget on SIGTERM                       |
| `PORT`                  | `8080`        | HTTP listen port                                       |
| `IS_LOCAL`              | unset         | When `true`, always emit debug response headers        |
| `LOCAL_MANIFEST_PATH`   | unset         | Load route table from this JSON file instead of Valkey |

### Local dev — direct Lambda invoke

Proxy mode uses the default `aws-config` credential chain. In-cluster
this resolves via IRSA on the `api-prime` ServiceAccount; locally,
provide one of:

- `AWS_PROFILE` — uses `~/.aws/credentials`
- `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` (+ `AWS_SESSION_TOKEN`)

Mount via the local-dev Kustomize overlay (`apps/k8s/api-prime/overlays/local/`).

### LOCAL_MANIFEST_PATH

Set `LOCAL_MANIFEST_PATH=/etc/api-prime/manifest.json` to skip the
Valkey route table entirely. The file is polled every 5s for mtime
changes and reloaded on diff. Useful for integration tests + local
dev where running the full controller is overkill. File shape: either
a top-level `[RouteEntry, ...]` array or `{"routes": [...]}`.

### Debug response headers

Emitted only when `IS_LOCAL=true` OR the request carries
`X-Smoo-Cache-Debug: 1`. Headers:

- `X-Smoo-Cache-Status` — `HIT | MISS | STALE | BYPASS`
- `X-Smoo-Cache-Key` — first 8 hex chars of the SHA-256 cache key
- `X-Smoo-Route-Mode` — `proxy | cache | implement`
- `X-Smoo-Lambda-Arn` — ARN actually invoked (empty for implement / cache-HIT)

In production these headers are never set.

## What's next

1. Wire EKS deployment manifests in the smooai monorepo at
   `apps/k8s/apps/api-prime/` and `apps/k8s/apps/api-prime-controller/`.
   Manifest needs: `EDGE_ATTEST_SECRET` env (from `@smooai/config`
   ExternalSecret), `IS_LOCAL` env in local overlay, `lambda:InvokeFunction`
   in the api-prime IRSA role (wildcard `function:smooai-production-*`
   for v1; per-route ARN tightening is a follow-up ticket).
2. Schema validation (SMOODEV-1277) — replace `src/edge/schema.rs` stub.
3. Hook into the shadow harness from Phase 6.
4. Fill in the controller (Wave 3): reconcile loop, admin endpoints,
   internal cache invalidation, Lambda health probing, OpenAPI emission.
