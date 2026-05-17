# @smooai/observability

Sentry-like error tracking for the Smoo AI platform — browser, Node, React, and Next.js. Captures unhandled exceptions, groups them server-side by stable fingerprint, attaches user/org/release context, and surfaces them in the Smoo dashboard.

[![npm version](https://img.shields.io/npm/v/@smooai/observability.svg)](https://www.npmjs.com/package/@smooai/observability)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Packages

| Package | npm | Purpose |
| --- | --- | --- |
| `@smooai/observability` | [![](https://img.shields.io/npm/v/@smooai/observability.svg)](https://www.npmjs.com/package/@smooai/observability) | Core client — browser + Node universal |
| `@smooai/observability-react` | [![](https://img.shields.io/npm/v/@smooai/observability-react.svg)](https://www.npmjs.com/package/@smooai/observability-react) | React `<ErrorBoundary>` + `useErrorHandler` |
| `@smooai/observability-next` | [![](https://img.shields.io/npm/v/@smooai/observability-next.svg)](https://www.npmjs.com/package/@smooai/observability-next) | Next.js wrapper + sourcemap upload |

## Quick start (Next.js)

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
        environment: process.env.NODE_ENV,
        release: process.env.GITHUB_SHA,
    });
}
```

```tsx
// app/global-error.tsx
'use client';
import { RootErrorBoundary } from '@smooai/observability-next';

export default function GlobalError({ error, reset }) {
    return (
        <html>
            <body>
                <RootErrorBoundary error={error} resetError={reset}>
                    <div>Something went wrong</div>
                </RootErrorBoundary>
            </body>
        </html>
    );
}
```

## Quick start (Browser SPA)

```ts
import { Client } from '@smooai/observability';

Client.init({
    dsn: process.env.SMOO_OBSERVABILITY_DSN!,
    environment: 'production',
    release: import.meta.env.VITE_GIT_SHA,
});

// Optional: identify the current user
Client.setUser({ id: 'user_abc', orgId: 'org_xyz' });
```

## Quick start (Node / Hono)

```ts
import { Client, observabilityMiddleware } from '@smooai/observability/node';

Client.init({
    dsn: process.env.OBSERVABILITY_INGEST_URL!,
    environment: process.env.STAGE!,
    release: process.env.LAMBDA_FUNCTION_VERSION ?? 'dev',
});

app.use('*', observabilityMiddleware());
```

## What gets captured

- Browser: `window.onerror`, `unhandledrejection`, `console.error` taps, `fetch` / `XHR` breadcrumbs, click + navigation breadcrumbs, React error boundaries
- Node: `uncaughtException`, `unhandledRejection`, Hono middleware over the global `onError` handler
- All: stack traces with sourcemap symbolication, user context, request context, release tag, environment, custom tags and scopes

## What does NOT get captured

- Anything sent through `console.log` / `console.info` / `console.warn` — only `console.error` is tapped, and that can be opted out
- HTTP request bodies (only method + path + status + duration in breadcrumbs)
- Anything matching the PII scrub regex: `password`, `token`, `authorization`, `Bearer ...`

## Status

`0.1.0` — initial release. Browser + Node + React + Next.js TypeScript SDKs. Source code in this repo; backend ingest + dashboard live in the [smooai monorepo](https://github.com/SmooAI/smooai). Follow-ups: Python, .NET, Go, Rust clients.

## License

MIT
