'use client';

import { Client } from '@smooai/observability';
import { ErrorBoundary } from '@smooai/observability-react';
import type { ReactNode } from 'react';
import { useEffect } from 'react';

interface RootErrorBoundaryProps {
    /** Error from Next.js global-error / error.tsx. */
    error: Error & { digest?: string };
    /** Next.js reset callback. */
    resetError: () => void;
    /** What to render in the error state. */
    fallback?: ReactNode;
    /** Optional children — usually omitted in global-error.tsx. */
    children?: ReactNode;
}

/**
 * Drop-in component for `app/global-error.tsx` and `app/error.tsx`.
 *
 * Captures the error to Smoo Observability on mount and renders the fallback.
 * If no fallback is provided, renders nothing (caller is expected to wrap with
 * its own UI).
 */
export function RootErrorBoundary({ error, resetError, fallback, children }: RootErrorBoundaryProps) {
    useEffect(() => {
        Client.captureException(error, {
            tags: {
                source: 'next-root-error-boundary',
                digest: error.digest ?? 'none',
            },
        });
    }, [error]);

    if (children) {
        return (
            <ErrorBoundary fallback={fallback ?? null}>
                {children}
            </ErrorBoundary>
        );
    }
    return <>{fallback ?? null}</>;
}

// Re-export the React bindings under this package as a convenience.
export { ErrorBoundary, useErrorHandler } from '@smooai/observability-react';
