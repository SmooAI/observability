<a name="readme-top"></a>

<p align="center">
  <a href="https://smoo.ai"><img src=".github/banner.png" alt="@smooai/observability — Error capture and grouping, your backend only." width="100%" /></a>
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/@smooai/observability"><img src="https://img.shields.io/npm/v/@smooai/observability?style=for-the-badge&color=00A6A6&label=npm&logo=npm&logoColor=white&labelColor=020618" alt="npm"></a>
  <a href="https://smoo.ai"><img src="https://img.shields.io/badge/Smoo_AI-platform-00A6A6?style=for-the-badge&labelColor=020618" alt="Smoo AI"></a>
  <img src="https://img.shields.io/badge/license-MIT-F49F0A?style=for-the-badge&labelColor=020618" alt="license">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/TypeScript-strict_ESM-00A6A6?style=flat-square" alt="TypeScript">
  <img src="https://img.shields.io/github/actions/workflow/status/SmooAI/observability/pr-checks.yml?style=flat-square&color=00A6A6&label=CI" alt="CI">
  <img src="https://img.shields.io/npm/dw/@smooai/observability?style=flat-square&color=F49F0A&label=downloads" alt="downloads">
</p>

<p align="center">
  <a href="#-features"><b>Features</b></a> &nbsp;·&nbsp; <a href="#-install"><b>Install</b></a> &nbsp;·&nbsp; <a href="#-usage"><b>Usage</b></a> &nbsp;·&nbsp; <a href="#-architecture"><b>Architecture</b></a> &nbsp;·&nbsp; <a href="#-part-of-smoo-ai"><b>Platform</b></a>
</p>

---

> The error-tracking platform we wished was already in our stack. You ship a deploy; somewhere out there a webpack chunk is 404'ing for one user and your sign-in page is silently broken. Your error boundary `console.error`s into the void, and your only signal is the support ticket that arrives forty minutes later. `@smooai/observability` fills that gap: automatic capture and grouping across every runtime, your events going to **your** Smoo backend only.

## ✨ Features

**Browser**

- 🛑 **Uncaught exceptions** — `window.onerror`, `unhandledrejection`, `console.error` taps
- 🍞 **Breadcrumbs** — `fetch` / `XHR` calls, click events, navigation events, custom traces
- 🧭 **Release tagging** — every event ships with the git sha so symbolication is one click away
- 🗺️ **Source maps** — uploaded to S3 at build time, applied lazily on view
- 🚪 **Beacon flush** — events queued at `pagehide` ship via `navigator.sendBeacon`
- 💾 **Offline queue** — events captured while offline persist in `IndexedDB` and retry on focus
- 🔐 **PII scrub** — credentials (`password`, `token`, `Bearer ...`) are dropped; emails / phones / addresses are HMAC-hashed per-org (`a@b.com` → `[email:9f2a41c8]`) so traces stay correlatable without storing the value

**Node**

- 🛑 **`uncaughtException` + `unhandledRejection`** with full stack
- 🪢 **Hono middleware** — captures errors propagating to the global `onError` handler
- 🧠 **AsyncLocalStorage scope** — per-request user, tags, breadcrumbs without leaking across requests
- 📦 **Batched transport** — `undici` with retry / backoff
- 🔐 **Same PII scrub policy** as the browser — key from `SMOOAI_OBSERVABILITY_PII_HASH_KEY`

**React / Next.js**

- 🧱 **`<ErrorBoundary>`** — drop-in component, captures and renders your fallback
- ⚓ **`useErrorHandler()`** — for async event-handler errors React boundaries can't see
- 🏗️ **`withSmooObservability(nextConfig)`** — enables production browser source maps and uploads them in CI
- 🛡️ **`<RootErrorBoundary>`** — drop into `app/global-error.tsx` / `app/error.tsx`

### What does NOT get captured

- `console.log` / `console.info` / `console.warn` — only `console.error` is tapped, and that's opt-out
- HTTP request **bodies** — only method, path, status, and duration appear in breadcrumbs
- Credentials matching the PII scrub regex — dropped outright, never hashed
- Raw emails / phones / street addresses — replaced by a keyed per-org hash, never stored in the clear

## 📦 Install

```sh
pnpm add @smooai/observability                      # core (browser + Node)
pnpm add @smooai/observability-react                # React bindings
pnpm add @smooai/observability-next                 # Next.js wrapper
```

or with npm / yarn / bun — same names.

| Package                                         | npm                                                                                                                                                          | Purpose                                     |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------- |
| [`@smooai/observability`](packages/core)        | [![npm](https://img.shields.io/npm/v/@smooai/observability?style=flat-square&color=00A6A6)](https://www.npmjs.com/package/@smooai/observability)             | Core client — browser + Node universal      |
| [`@smooai/observability-react`](packages/react) | [![npm](https://img.shields.io/npm/v/@smooai/observability-react?style=flat-square&color=00A6A6)](https://www.npmjs.com/package/@smooai/observability-react) | React `<ErrorBoundary>` + `useErrorHandler` |
| [`@smooai/observability-next`](packages/next)   | [![npm](https://img.shields.io/npm/v/@smooai/observability-next?style=flat-square&color=00A6A6)](https://www.npmjs.com/package/@smooai/observability-next)   | Next.js wrapper + sourcemap upload          |

## 🚀 Usage

### Next.js

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

### Browser SPA

```ts
import { Client } from '@smooai/observability';

Client.init({
    dsn: process.env.SMOO_OBSERVABILITY_DSN!,
    environment: 'production',
    release: import.meta.env.VITE_GIT_SHA,
});

Client.setUser({ id: 'user_abc', orgId: 'org_xyz' });
```

### Node / Hono

```ts
import { Client, observabilityMiddleware } from '@smooai/observability/node';

Client.init({
    dsn: process.env.OBSERVABILITY_INGEST_URL!,
    environment: process.env.STAGE!,
    release: process.env.LAMBDA_FUNCTION_VERSION ?? 'dev',
});

app.use('*', observabilityMiddleware());
```

### Multi-language support

The same ingest contract (`POST /webhooks/observability/{org_id}/{token}` with `type: 'error'`) accepts events from any language. Follow-up SDKs:

- 🐍 **Python** — `smooai-observability` on PyPI (tracked in SMOODEV-1067 follow-ups)
- 🦀 **Rust** — `smooai-observability` crate (tracked in SMOODEV-1067 follow-ups)
- 🐹 **Go** — `github.com/smooai/observability-go` (tracked in SMOODEV-1067 follow-ups)
- 💠 **.NET** — `SmooAI.Observability` on NuGet (tracked in SMOODEV-1067 follow-ups)

### 🤖 GenAI telemetry (`gen_ai.*`)

LLM and agent spans carry the [OTel GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/), so any semconv-aware backend reads them — Smoo's LLM dashboard routes on `gen_ai.system` alone.

```ts
import OpenAI from 'openai';
import { wrapOpenAI } from '@smooai/observability';

// Instruments chat.completions.create — the original client is untouched.
const openai = wrapOpenAI(new OpenAI(), {
    conversationId: conversation.id,
    // Providers don't return a price. Supply one and the cost column fills in.
    costUsd: ({ inputTokens = 0, outputTokens = 0 }) => inputTokens * 2.5e-6 + outputTokens * 1e-5,
});
```

The same wrapper covers Groq, Together, Fireworks, DeepSeek, Azure OpenAI, and any OpenAI-compatible gateway — pass `{ system: 'groq' }` so spans attribute to the real provider. Prompt and completion **content is off by default**; `{ recordContent: true }` records it as `gen_ai.*.message` span events, PII-scrubbed on the way out.

For hand-rolled calls, set the attributes directly:

```ts
import { setGenAIAttributes, recordGenAIMessage } from '@smooai/observability';

setGenAIAttributes(span, { system: 'anthropic', operationName: 'chat', requestModel: 'claude-opus-4-7', usageInputTokens: 812, usageOutputTokens: 96 });
```

> `gen_ai.operation.name` is a straight passthrough on ingest with **no fallback** — leave it unset and the operation column lands `NULL`. Always set it.

**Parity across the five SDKs:**

| SDK            | Attribute helper              | Message events                | Content PII-scrubbed | Framework integration                               |
| -------------- | ----------------------------- | ----------------------------- | -------------------- | --------------------------------------------------- |
| **TypeScript** | `setGenAIAttributes`          | `recordGenAIMessage`          | ✅                   | ✅ `wrapOpenAI` — OpenAI Node SDK + compatible APIs |
| **Rust**       | `set_gen_ai_attributes`       | `record_gen_ai_message`       | ❌                   | —                                                   |
| **Python**     | `set_gen_ai_attributes`       | `record_gen_ai_message`       | ❌                   | ✅ `SmooAICallbackHandler` — LangChain / LangGraph  |
| **Go**         | `SetGenAIAttributes`          | `RecordGenAIMessage`          | ❌                   | —                                                   |
| **.NET**       | `GenAIActivity.SetAttributes` | `GenAIActivity.RecordMessage` | ❌                   | —                                                   |

Known divergences: TypeScript, Python, Go and .NET emit `gen_ai.tool.names` as a **string array**; Rust emits a comma-joined string. Only TypeScript scrubs recorded message content today.

## 📖 Architecture

The SDK is intentionally thin. It captures, batches, redacts credentials, hashes personal identifiers, and POSTs to a Smoo ingest endpoint. All of the heavy lifting — fingerprint grouping, source-map symbolication, dashboards, alerts, retention — lives in the Smoo platform.

```mermaid
%%{init: {'theme':'base','themeVariables':{
  'background':'#020618','primaryColor':'#0b1426','primaryTextColor':'#e6edf6','primaryBorderColor':'#2b3a52',
  'lineColor':'#7c8aa0','secondaryColor':'#0b1426','tertiaryColor':'#0b1426','fontFamily':'ui-sans-serif, system-ui, sans-serif',
  'clusterBkg':'#0b1426','clusterBorder':'#22304a'}}}%%
flowchart LR
  REACT["observability-react<br/>ErrorBoundary · useErrorHandler"] -->|wraps| CORE
  NEXT["observability-next<br/>withSmooObservability · sourcemaps"] -->|wraps| CORE
  CORE["@smooai/observability<br/>capture · scope · scrub · batch"]
  CORE -->|"POST /webhooks/observability/{org}/{token}<br/>Bearer B2M JWT · gzipped JSON"| INGEST[("Smoo platform<br/>group · symbolicate · alert")]

  classDef warm fill:#f49f0a,stroke:#ff6b6c,color:#1a0f00;
  classDef teal fill:#00a6a6,stroke:#00c2c2,color:#011;
  class CORE warm
  class INGEST teal
```

Full backend architecture: [SmooAI/smooai → docs/Architecture/Observability-Architecture.md](https://github.com/SmooAI/smooai/blob/main/docs/Architecture/Observability-Architecture.md).

## 📖 Built with

- **TypeScript** — strict mode, ESM-only, dual browser/Node entries via package `exports` map
- **tsup** — bundling, dual ESM/types output, sourcemaps
- **turborepo** — fast pipeline across the three packages
- **vitest** — unit tests
- **changesets** — versioning + npm publish via GitHub Actions

## 📖 Privacy & telemetry

This SDK is opinionated about privacy:

- We never capture form bodies, request bodies, or response bodies by default
- We never capture cookies
- We never send anything to a third-party service — your events go to **your** Smoo backend only
- PII scrubbing is enabled by default and can be tuned per-tenant. Personal identifiers are hashed with HMAC-SHA256 under a key you supply (`SMOOAI_OBSERVABILITY_PII_HASH_KEY`), salted by org id — identical across the TypeScript, Rust, Go, Python and .NET SDKs. **With no key configured they are fully redacted, never hashed under a guessable one.**

## 📖 Status

`0.1.0` — types and client skeleton are stable. The capture handlers, stack parsers, transport, and source-map upload land incrementally in upcoming `0.x` releases. The backend ingest, fingerprint grouping, dashboard, and customer-org rollout live in the [SmooAI/smooai monorepo](https://github.com/SmooAI/smooai) and are tracked under [SMOODEV-1067](https://smooai.atlassian.net/browse/SMOODEV-1067).

## 🧩 Part of Smoo AI {#part-of-smoo-ai}

`@smooai/observability` is built and open-sourced by **[Smoo AI](https://smoo.ai)** — the AI-powered business platform with AI built into every product: CRM, customer support, campaigns, field service, observability, and developer tools.

- 🚀 **Observability on the platform** — [smoo.ai/platform/observability](https://smoo.ai/platform/observability)
- 🧰 **More open source from Smoo AI** — [smoo.ai/open-source](https://smoo.ai/open-source)
- 🧩 **Sibling packages** — [@smooai/logger](https://github.com/SmooAI/logger), [@smooai/config](https://github.com/SmooAI/config), [@smooai/fetch](https://github.com/SmooAI/fetch), [smooth](https://github.com/SmooAI/smooth) (the `th` CLI)

## 🤝 Contributing

Issues and PRs welcome. Maintained by Brent Rager — [email](mailto:brent@smoo.ai) · [LinkedIn](https://www.linkedin.com/in/brentrager/) · [BlueSky](https://bsky.app/profile/brentragertech.bsky.social) · [TikTok](https://www.tiktok.com/@brentragertech) · [Instagram](https://www.instagram.com/brentragertech/).

## 📄 License

MIT © Smoo AI, Inc. See [LICENSE](LICENSE).

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

<p align="center">
  Built by <a href="https://smoo.ai"><strong>Smoo AI</strong></a> — AI built into every product.
</p>
