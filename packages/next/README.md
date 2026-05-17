# @smooai/observability-next

Next.js wrapper for `@smooai/observability`. Provides:

- `<RootErrorBoundary>` for `app/global-error.tsx` and `app/error.tsx`
- `withSmooObservability(nextConfig, options)` for `next.config.ts` (build-time sourcemap upload, productionBrowserSourceMaps)

```
npm i @smooai/observability @smooai/observability-react @smooai/observability-next
```

## Usage

### `next.config.ts`

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

### `app/global-error.tsx`

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
