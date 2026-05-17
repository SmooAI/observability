import { Client } from '@smooai/observability';
import { Component, type ErrorInfo, type ReactNode, useCallback } from 'react';

interface ErrorBoundaryProps {
    fallback: ReactNode | ((args: { error: Error; resetError: () => void }) => ReactNode);
    onError?: (error: Error, info: ErrorInfo) => void;
    children: ReactNode;
}

interface ErrorBoundaryState {
    error: Error | null;
}

/**
 * React ErrorBoundary that reports caught errors to @smooai/observability
 * and renders a fallback. The fallback may be static or a function that
 * receives a `resetError` callback.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
    override state: ErrorBoundaryState = { error: null };

    static getDerivedStateFromError(error: Error): ErrorBoundaryState {
        return { error };
    }

    override componentDidCatch(error: Error, info: ErrorInfo): void {
        Client.captureException(error, { tags: { source: 'react-error-boundary' } });
        this.props.onError?.(error, info);
    }

    resetError = (): void => {
        this.setState({ error: null });
    };

    override render(): ReactNode {
        if (this.state.error) {
            const f = this.props.fallback;
            if (typeof f === 'function') {
                return f({ error: this.state.error, resetError: this.resetError });
            }
            return f;
        }
        return this.props.children;
    }
}

/**
 * Hook for capturing errors raised inside async event handlers — places
 * React's render-time boundary can't see.
 */
export function useErrorHandler(): (err: unknown) => void {
    return useCallback((err: unknown) => {
        Client.captureException(err, { tags: { source: 'react-use-error-handler' } });
    }, []);
}
