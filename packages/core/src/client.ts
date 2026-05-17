import { getCurrentScope } from './scope';
import type { ClientOptions, ExceptionInfo, Level, ObservabilityEvent, Runtime, StackFrame } from './types';

const SDK_NAME = '@smooai/observability';
const SDK_VERSION = '0.1.0';

/**
 * Singleton client used by both browser and Node entry points. The transport
 * + capture-handler integrations are wired in by the runtime-specific entry
 * (`src/browser/index.ts`, `src/node/index.ts`).
 *
 * This file is intentionally minimal until the SDK implementation lands —
 * see SMOODEV-1067 follow-up pearls.
 */
class _Client {
    private options: ClientOptions | null = null;
    private runtime: Runtime = typeof window === 'undefined' ? 'node' : 'browser';
    private transport: ((batch: ObservabilityEvent[]) => Promise<void>) | null = null;

    init(options: ClientOptions): void {
        this.options = options;
        // Capture-handler registration happens in the runtime entry point
        // (browser/index.ts or node/index.ts), which calls `_registerTransport`
        // and binds globals like window.onerror / process events.
    }

    _isInitialized(): boolean {
        return this.options !== null;
    }

    _getOptions(): ClientOptions | null {
        return this.options;
    }

    _registerTransport(t: (batch: ObservabilityEvent[]) => Promise<void>): void {
        this.transport = t;
    }

    setUser(user: ObservabilityEvent['user']): void {
        getCurrentScope().setUser(user);
    }
    setTag(k: string, v: string): void {
        getCurrentScope().setTag(k, v);
    }
    addBreadcrumb(category: string, message?: string, data?: Record<string, unknown>, level: Level = 'info'): void {
        getCurrentScope().addBreadcrumb({ category, message, data, level, timestamp: Date.now() });
    }

    captureException(error: unknown, extra?: { tags?: Record<string, string> }): string | undefined {
        if (!this.options) return undefined;
        const eventId = crypto.randomUUID();
        const e = toException(error);
        const event: ObservabilityEvent = getCurrentScope().applyToEvent({
            eventId,
            timestamp: Date.now(),
            level: 'error',
            exception: [e],
            tags: extra?.tags,
            release: this.options.release,
            environment: this.options.environment,
            sdk: { name: SDK_NAME, version: SDK_VERSION, runtime: this.runtime },
        });
        const final = this.options.beforeSend ? this.options.beforeSend(event) : event;
        if (final && this.transport) {
            // Fire-and-forget; transport handles batching/retry.
            void this.transport([final]).catch(() => {
                /* swallow — observability must not throw */
            });
        }
        return eventId;
    }

    captureMessage(message: string, level: Level = 'info'): string | undefined {
        if (!this.options) return undefined;
        const eventId = crypto.randomUUID();
        const event: ObservabilityEvent = getCurrentScope().applyToEvent({
            eventId,
            timestamp: Date.now(),
            level,
            message,
            release: this.options.release,
            environment: this.options.environment,
            sdk: { name: SDK_NAME, version: SDK_VERSION, runtime: this.runtime },
        });
        const final = this.options.beforeSend ? this.options.beforeSend(event) : event;
        if (final && this.transport) {
            void this.transport([final]).catch(() => {});
        }
        return eventId;
    }
}

function toException(err: unknown): ExceptionInfo {
    // Minimal conversion — full stack-frame parsing lives in runtime modules.
    if (err instanceof Error) {
        return {
            type: err.name,
            value: err.message,
            stacktrace: { frames: parseStackString(err.stack) },
        };
    }
    return {
        type: 'Unknown',
        value: typeof err === 'string' ? err : JSON.stringify(err),
        stacktrace: { frames: [] },
    };
}

function parseStackString(stack: string | undefined): StackFrame[] {
    if (!stack) return [];
    // Real parser lives in runtime entries (different stack formats per engine).
    // Here, return one synthetic frame so the event is well-formed.
    return [{ module: 'unparsed', inApp: true }];
}

export const Client = new _Client();
export type { _Client };
