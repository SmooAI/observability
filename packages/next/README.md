<a name="readme-top"></a>

<br />
<div align="center">
  <a href="https://smoo.ai">
    <img src="../../images/logo.png" alt="SmooAI Logo" />
  </a>
</div>

# @smooai/observability-next

![NPM Version](https://img.shields.io/npm/v/%40smooai%2Fobservability-next?style=for-the-badge)
![NPM Downloads](https://img.shields.io/npm/dw/%40smooai%2Fobservability-next?style=for-the-badge)
![GitHub License](https://img.shields.io/github/license/SmooAI/observability?style=for-the-badge)

Next.js wrapper for `@smooai/observability`. One config wrap, one boundary, automatic source maps, full release tagging.

```sh
pnpm add @smooai/observability @smooai/observability-react @smooai/observability-next
```

## API

### `withSmooObservability(nextConfig, options)`

Wrap your `next.config.ts`. Enables `productionBrowserSourceMaps` and (in CI) uploads them to the Smoo Observability sourcemap bucket so stacks are symbolicated automatically.

```ts
import { withSmooObservability } from '@smooai/observability-next/build';

export default withSmooObservability(
    {
        /* your Next config */
    },
    {
        org: 'your-org',
        release: process.env.GITHUB_SHA ?? 'dev',
        uploadSourcemaps: process.env.CI === 'true',
    },
);
```

### `<RootErrorBoundary>`

```tsx
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

### `instrumentation.ts`

```ts
export async function register() {
    const { Client } = await import('@smooai/observability');
    Client.init({
        dsn: process.env.OBSERVABILITY_INGEST_URL!,
        environment: process.env.STAGE,
        release: process.env.GITHUB_SHA ?? 'dev',
    });
}
```

## License

MIT
