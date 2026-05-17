<a name="readme-top"></a>

<!-- PROJECT LOGO -->
<br />
<div align="center">
  <a href="https://smoo.ai">
    <img src="images/logo.png" alt="SmooAI Logo" />
  </a>
</div>

<!-- ABOUT THE PROJECT -->

## About Smoo AI

**[Smoo AI](https://smoo.ai)** is an AI platform that helps businesses multiply their customer, employee, and developer experience — conversational AI for support and sales, paired with the production-grade developer tooling we use to build it.

This library is part of a small family of open-source packages we maintain to keep our own stack honest: structured logging, typed HTTP, file storage, configuration, and — now — error tracking. Use them in your stack, or take them as a reference for how we build.

- 🌐 [smoo.ai](https://smoo.ai) — the product
- 📦 [smoo.ai/open-source](https://smoo.ai/open-source) — every open-source package we ship
- 🐙 [github.com/SmooAI](https://github.com/SmooAI) — the source

## About @smooai/observability

**The error-tracking platform we wished was already in our stack.** Sentry-style capture and grouping for the browser and Node, with first-class wrappers for React and Next.js — built on the same SmooAI conventions you already know.

![NPM Version](https://img.shields.io/npm/v/%40smooai%2Fobservability?style=for-the-badge)
![NPM Downloads](https://img.shields.io/npm/dw/%40smooai%2Fobservability?style=for-the-badge)
![NPM Last Update](https://img.shields.io/npm/last-update/%40smooai%2Fobservability?style=for-the-badge)

![GitHub License](https://img.shields.io/github/license/SmooAI/observability?style=for-the-badge)
![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/SmooAI/observability/pr-checks.yml?style=for-the-badge)
![GitHub Repo stars](https://img.shields.io/github/stars/SmooAI/observability?style=for-the-badge)

### Why @smooai/observability?

You've shipped a deploy. Somewhere out there a webpack chunk is 404'ing for one user and your sign-in page is silently broken. Your error boundary console.errors into the void. Your only signal is the support ticket that arrives forty minutes later.

That's the gap this package fills.

**Captured automatically, on every runtime:**

**Browser**

- 🛑 **Uncaught exceptions** — `window.onerror`, `unhandledrejection`, `console.error` taps
- 🍞 **Breadcrumbs** — `fetch` / `XHR` calls, click events, navigation events, custom traces
- 🧭 **Release tagging** — every event ships with the git sha so symbolication is one click away
- 🗺️ **Source maps** — uploaded to S3 at build time, applied lazily on view
- 🚪 **Beacon flush** — events queued at `pagehide` ship via `navigator.sendBeacon`
- 💾 **Offline queue** — events captured while offline persist in `IndexedDB` and retry on focus
- 🔐 **PII scrub** — `password`, `token`, `Bearer ...`, and friends are redacted before transport

**Node**

- 🛑 **`uncaughtException` + `unhandledRejection`** with full stack
- 🪢 **Hono middleware** — captures errors propagating to the global `onError` handler
- 🧠 **AsyncLocalStorage scope** — per-request user, tags, breadcrumbs without leaking across requests
- 📦 **Batched transport** — `undici` with retry / backoff
- 🔐 **Same PII scrub policy** as the browser

**React / Next.js**

- 🧱 **`<ErrorBoundary>`** — drop-in component, captures and renders your fallback
- ⚓ **`useErrorHandler()`** — for async event-handler errors React boundaries can't see
- 🏗️ **`withSmooObservability(nextConfig)`** — enables production browser source maps and uploads them in CI
- 🛡️ **`<RootErrorBoundary>`** — drop into `app/global-error.tsx` / `app/error.tsx`

### Install

```sh
pnpm add @smooai/observability                      # core (browser + Node)
pnpm add @smooai/observability-react                # React bindings
pnpm add @smooai/observability-next                 # Next.js wrapper
```

or with npm / yarn / bun — same names.

### Quick Start (Next.js)

```ts
// next.config.ts
import { withSmooObservability } from '@smooai/observability-next/build';

export default withSmooObservability(
    {
        /* your config */
    },
    {
        org: 'your-org',
        release: process.env.GITHUB_SHA ?? 'dev',
        uploadSourcemaps: process.env.CI === 'true',
    },
);
```

```ts
// instrumentation.ts
export async function register() {
    const { Client } = await import('@smooai/observability');
    Client.init({
        dsn: process.env.OBSERVABILITY_INGEST_URL!,
        environment: process.env.STAGE,
        release: process.env.GITHUB_SHA ?? 'dev',
    });
}
```

```tsx
// app/global-error.tsx
'use client';
import { RootErrorBoundary } from '@smooai/observability-next';

export default function GlobalError({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
    return (
        <html>
            <body>
                <RootErrorBoundary error={error} resetError={reset} fallback={<YourBrandedError onRetry={reset} />} />
            </body>
        </html>
    );
}
```

### Quick Start (Browser SPA)

```ts
import { Client } from '@smooai/observability';

Client.init({
    dsn: process.env.SMOO_OBSERVABILITY_DSN!,
    environment: 'production',
    release: import.meta.env.VITE_GIT_SHA,
});

Client.setUser({ id: 'user_abc', orgId: 'org_xyz' });
```

### Quick Start (Node / Hono)

```ts
import { Client, observabilityMiddleware } from '@smooai/observability/node';

Client.init({
    dsn: process.env.OBSERVABILITY_INGEST_URL!,
    environment: process.env.STAGE!,
    release: process.env.LAMBDA_FUNCTION_VERSION ?? 'dev',
});

app.use('*', observabilityMiddleware());
```

### What Does NOT Get Captured

- `console.log` / `console.info` / `console.warn` — only `console.error` is tapped, and that's opt-out
- HTTP request **bodies** — only method, path, status, and duration appear in breadcrumbs
- Anything matching the PII scrub regex unless you explicitly allowlist it

### Packages

| Package                                         | npm                                                                                                                                       | Purpose                                     |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| [`@smooai/observability`](packages/core)        | [![NPM Version](https://img.shields.io/npm/v/%40smooai%2Fobservability)](https://www.npmjs.com/package/@smooai/observability)             | Core client — browser + Node universal      |
| [`@smooai/observability-react`](packages/react) | [![NPM Version](https://img.shields.io/npm/v/%40smooai%2Fobservability-react)](https://www.npmjs.com/package/@smooai/observability-react) | React `<ErrorBoundary>` + `useErrorHandler` |
| [`@smooai/observability-next`](packages/next)   | [![NPM Version](https://img.shields.io/npm/v/%40smooai%2Fobservability-next)](https://www.npmjs.com/package/@smooai/observability-next)   | Next.js wrapper + sourcemap upload          |

### Multi-Language Support

The same ingest contract (`POST /webhooks/observability/{org_id}/{token}` with `type: 'error'`) accepts events from any language. Follow-up SDKs:

- 🐍 **Python** — `smooai-observability` on PyPI (tracked in SMOODEV-1067 follow-ups)
- 🦀 **Rust** — `smooai-observability` crate (tracked in SMOODEV-1067 follow-ups)
- 🐹 **Go** — `github.com/smooai/observability-go` (tracked in SMOODEV-1067 follow-ups)
- 💠 **.NET** — `SmooAI.Observability` on NuGet (tracked in SMOODEV-1067 follow-ups)

## Architecture

The SDK is intentionally thin. It captures, batches, redacts PII, and POSTs to a Smoo ingest endpoint. All of the heavy lifting — fingerprint grouping, source-map symbolication, dashboards, alerts, retention — lives in the Smoo platform.

```
┌──────────────────────────┐    POST /webhooks/observability/{org}/{token}
│ @smooai/observability    │ ────────────────────────────────────────────▶
│   ├─ capture handlers    │                  (Bearer B2M JWT, gzipped JSON)
│   ├─ Scope / breadcrumbs │
│   ├─ batched transport   │
│   └─ PII scrub           │
└──────────────────────────┘
            ▲
            │ wraps
┌──────────────────────────┐      ┌──────────────────────────────────┐
│ @smooai/observability-   │      │ @smooai/observability-next       │
│   react                  │      │   ├─ withSmooObservability()     │
│   ├─ <ErrorBoundary>     │      │   ├─ RootErrorBoundary           │
│   └─ useErrorHandler()   │      │   └─ build-time sourcemap upload │
└──────────────────────────┘      └──────────────────────────────────┘
```

Full backend architecture: [SmooAI/smooai → docs/Architecture/Observability-Architecture.md](https://github.com/SmooAI/smooai/blob/main/docs/Architecture/Observability-Architecture.md).

## Built With

- **TypeScript** — strict mode, ESM-only, dual browser/Node entries via package `exports` map
- **tsup** — bundling, dual ESM/types output, sourcemaps
- **turborepo** — fast pipeline across the three packages
- **vitest** — unit tests
- **changesets** — versioning + npm publish via GitHub Actions

## Privacy & Telemetry

This SDK is opinionated about privacy:

- We never capture form bodies, request bodies, or response bodies by default
- We never capture cookies
- We never send anything to a third-party service — your events go to **your** Smoo backend only
- PII scrubbing is enabled by default and can be tuned per-tenant

## Status

`0.1.0` — types and client skeleton are stable. The capture handlers, stack parsers, transport, and source-map upload land incrementally in upcoming `0.x` releases. The backend ingest, fingerprint grouping, dashboard, and customer-org rollout live in the [SmooAI/smooai monorepo](https://github.com/SmooAI/smooai) and are tracked under [SMOODEV-1067](https://smooai.atlassian.net/browse/SMOODEV-1067).

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contact

Brent Rager

- [Email](mailto:brent@smoo.ai)
- [LinkedIn](https://www.linkedin.com/in/brentrager/)
- [BlueSky](https://bsky.app/profile/brentragertech.bsky.social)
- [TikTok](https://www.tiktok.com/@brentragertech)
- [Instagram](https://www.instagram.com/brentragertech/)

Smoo Github: [https://github.com/SmooAI](https://github.com/SmooAI)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## License

MIT © Smoo AI, Inc.
