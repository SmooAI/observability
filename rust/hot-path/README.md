# smooai-hot-path

Rust hot-path read API for SmooAI. Owns the high-traffic sidebar endpoints
that are too cold-start-prone on Lambda. Designed to run on EKS alongside
LiteLLM and Voice (Phases 3–5 of the EKS migration plan).

This crate is the **SMOODEV-1227 scaffold** — Phase 5c of the plan in
`~/.claude/plans/indexed-tinkering-hummingbird.md`. The follow-up Wave 2
agent fills in `POST /v1/auth/sign-in` and the remaining `/v1/*` reads.

## Endpoints

| Method | Path                  | Status   | Notes                                                                |
| ------ | --------------------- | -------- | -------------------------------------------------------------------- |
| GET    | `/health/liveness`    | working  | process alive                                                        |
| GET    | `/health/readiness`   | working  | pings Postgres + Redis                                               |
| GET    | `/v1/profile`         | working  | port of `packages/backend/src/routes/profile.ts` (GET)               |
| POST   | `/v1/auth/sign-in`    | working  | Supabase password-grant passthrough, rate-limited 10/60s per IP      |
| POST   | `/v1/auth/refresh`    | working  | Supabase refresh-token-grant passthrough                             |

`GET /v1/profile` requires `Authorization: Bearer <supabase-jwt>`.

### Auth endpoints

Both auth endpoints proxy to Supabase GoTrue (`{SUPABASE_URL}/auth/v1/token`)
and return the upstream JSON body **verbatim** on success — the dashboard
consumes `{access_token, refresh_token}` and hands them to
`@supabase/ssr`'s `setSession()` to write cookies in the standard
`sb-{project-ref}-auth-token` format.

Error mapping:

| Upstream                 | Returned to caller                              |
| ------------------------ | ----------------------------------------------- |
| 200 OK                   | 200 with verbatim GoTrue body                   |
| 400/401 invalid_grant    | 401 `{"message": "Invalid login credentials"}`  |
| 429                      | 429 `{"message": "Too many sign-in attempts."}` |
| 5xx / network error      | 502 `{"message": "Auth provider unavailable"}`  |

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

| Var                 | Required | Default                                              | Description                              |
| ------------------- | -------- | ---------------------------------------------------- | ---------------------------------------- |
| `DATABASE_URL`      | yes      | —                                                    | Postgres connection string               |
| `SUPABASE_URL`      | yes      | —                                                    | e.g. `https://xrqbqgotghitcfuoukdk.supabase.co` |
| `SUPABASE_ANON_KEY` | yes      | —                                                    | used by `/v1/auth/sign-in` + `/v1/auth/refresh` |
| `SUPABASE_JWKS_URL` | no       | `${SUPABASE_URL}/auth/v1/.well-known/jwks.json`      | JWKS endpoint for JWT verification       |
| `REDIS_URL`         | no       | `redis://127.0.0.1:6379`                             | Redis for read-through cache             |
| `PORT`              | no       | `3000`                                               | HTTP listen port                         |
| `RUST_LOG`          | no       | `info`                                               | tracing env filter                       |

## Local development

```bash
# From the repo root (observability/)
cd rust

# Build (uses the workspace at rust/Cargo.toml)
cargo build -p smooai-hot-path

# Release build
cargo build --release -p smooai-hot-path

# Run tests
cargo test -p smooai-hot-path

# Lint
cargo clippy -p smooai-hot-path --all-targets -- -D warnings

# Run it (requires a reachable Postgres + Supabase)
DATABASE_URL=postgres://postgres@127.0.0.1:54322/postgres \
SUPABASE_URL=https://xrqbqgotghitcfuoukdk.supabase.co \
SUPABASE_ANON_KEY=<anon-key> \
cargo run -p smooai-hot-path
```

The integration tests in `tests/integration_profile.rs` use
`sqlx::PgPool::connect_lazy`, so they pass without a live database — they
exercise the routing + auth-header parsing only.

## Docker

The Dockerfile lives at `rust/hot-path/Dockerfile` but its build context
must be the workspace root `rust/`, because cargo needs to see the
sibling members listed in `rust/Cargo.toml`.

```bash
# From observability repo root
docker buildx build \
    --platform linux/arm64 \
    -t ghcr.io/smooai/hot-path:dev \
    -f rust/hot-path/Dockerfile \
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

## What's next

1. Add the remaining `/v1/*` read endpoints listed in Phase 5c of the
   plan: `/v1/organizations`, `/v1/organizations/:id/features`,
   `/v1/organizations/:id/products`, and the batched `/me/bootstrap`.
2. Wire EKS deployment manifests in the smooai monorepo at
   `apps/k8s/apps/hot-path/`.
3. Hook into the shadow harness from Phase 6.
