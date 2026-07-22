<a name="readme-top"></a>

<br />
<div align="center">
  <a href="https://smoo.ai">
    <img src="../../images/logo.png" alt="SmooAI Logo" />
  </a>
</div>

# @smooai/observability

![NPM Version](https://img.shields.io/npm/v/%40smooai%2Fobservability?style=for-the-badge)
![NPM Downloads](https://img.shields.io/npm/dw/%40smooai%2Fobservability?style=for-the-badge)
![NPM Last Update](https://img.shields.io/npm/last-update/%40smooai%2Fobservability?style=for-the-badge)

![GitHub License](https://img.shields.io/github/license/SmooAI/observability?style=for-the-badge)
![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/SmooAI/observability/pr-checks.yml?style=for-the-badge)

Universal browser + Node SDK for Smoo AI Observability. Captures unhandled exceptions, builds a Scope with breadcrumbs and user context, redacts PII, and ships batched events to a Smoo ingest endpoint.

```sh
pnpm add @smooai/observability
```

## Entry points

| Import                          | Runtime                                    |
| ------------------------------- | ------------------------------------------ |
| `@smooai/observability`         | Auto-resolved by bundler (browser or node) |
| `@smooai/observability/browser` | Force browser entry                        |
| `@smooai/observability/node`    | Force Node entry                           |

## API

### `Client.init(options)`

```ts
import { Client } from '@smooai/observability';

Client.init({
    dsn: 'https://api.smoo.ai/webhooks/observability/<org>/<token>',
    environment: 'production',
    release: 'apps/web@abc1234',
    flushIntervalMs: 1000,
    maxBatchSize: 30,
    beforeSend: (event) => (event.tags?.skip ? null : event),
});
```

### Capture

```ts
Client.captureException(new Error('boom'), { tags: { vendor: 'flaky-co' } });
Client.captureMessage('user reached impossible state', 'warning');
```

### Scope

```ts
import { withScope, Client } from '@smooai/observability';

withScope((scope) => {
    scope.setTag('checkout-step', 'shipping');
    scope.addBreadcrumb({ category: 'custom', message: 'started shipping form', level: 'info', timestamp: Date.now() });
    // Anything captured inside the closure inherits these.
    Client.captureException(err);
});
```

### Breadcrumbs

```ts
Client.addBreadcrumb('fetch', 'POST /api/checkout 502', { method: 'POST', status: 502 }, 'error');
```

### User context

```ts
Client.setUser({ id: 'user_abc', orgId: 'org_xyz', sessionId: 'sess_123' });
```

### Sampling and telemetry settings (ADR-097)

Browser logs are sampled **by session, never by line** — the decision is made
once per session (or inherited from the trace, where one exists) and applies to
every line under it, so any trace you can open has 100% of its log lines.
Warnings and errors are always kept. Server-side logs are not sampled.

```ts
import { loadTelemetrySettings, sampleDecision, shouldEmitLog } from '@smooai/observability';

// Settings come from @smooai/config public-tier keys. The SDK never imports
// the config client — you inject a provider, so it stays usable offline.
const settings = await loadTelemetrySettings(() => publicConfig.getAll());
// ...unreachable / malformed / out-of-range → compiled-in ADR-010 defaults.
// Never "sample everything out".

shouldEmitLog({ level: 'info', sessionId, ...settings, minimumLevel: settings.minimumLogLevel, logSamplingRatio: settings.browserLogSamplingRatio });
```

`sampleDecision(id, ratio)` is FNV-1a 32-bit over the UTF-8 bytes of the id —
deterministic, stable for a page's lifetime, and reproduced byte-identically by
the Rust / Python / Go / .NET SDKs against
[`parity/sampling-corpus.json`](../../parity/README.md).

## What it does NOT do

- Does not capture `console.log` / `console.info` / `console.warn`
- Does not capture request / response bodies
- Does not capture cookies
- Does not contact any third-party

## Status

`0.1.0` — types and Client API are stable. Capture handlers and full transport ship in upcoming releases (see [SmooAI/smooai SMOODEV-1067](https://github.com/SmooAI/smooai)).

## License

MIT
