---
'@smooai/observability': minor
---

Node SDK capture handlers + Hono middleware (SMOODEV-1067 follow-up th-bafeb7).

`@smooai/observability/node` now ships real implementations:

- `registerNodeGlobalHandlers({ flush, exitOnUncaught })` — attaches `uncaughtException` + `unhandledRejection` listeners that forward to `Client.captureException`, plus optional SIGTERM / SIGINT / `beforeExit` flushing so a Lambda container shutdown drains the in-memory queue. Idempotent.
- `makeNodeTransport(options)` — Node-flavored `Transport` adapter (fetch + keepalive, no Beacon). Returns the underlying transport so callers (and the auto-init wiring) can hook the flush method into the lifecycle.
- `observabilityMiddleware({ resolveUser, requestHeaderAllowlist })` — Hono-shaped middleware. Per request: hydrates the active `Scope` with the authenticated user (defaults to reading `c.get('auth')` produced by `@smooai/auth`), adds a `request` context with method/path and an allow-listed header subset, wraps the handler chain in `withScope` so any `captureException` fired from a downstream handler picks up that request's identity, and captures thrown errors before re-throwing so Hono's onError still gets to render the response.
- `Client.init` on node now auto-wires the transport and global handlers (override with `autoInstrumentation: false`).

Also fixed a latent bug in `withScope`: previously the scope was popped before any `await` inside the callback resolved, so request-scoped state was gone by the time async handlers ran. `withScope` now defers the pop until a returned thenable settles, while keeping the synchronous fast path unchanged.

24 tests total (was 13). Build + typecheck clean.
