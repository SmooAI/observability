import { getCurrentScope } from './scope';
import { dropSdkFrames, parseStack } from './stack-parser';
import type { ClientOptions, ExceptionInfo, Level, ObservabilityEvent, Runtime, StackFrame } from './types';

const SDK_NAME = '@smooai/observability';
const SDK_VERSION = '0.1.0';

/**
 * Native per-runtime capture handler. When registered, `captureException` /
 * `captureMessage` route the prepared event through it and SKIP the HTTP
 * transport — used by the node runtime to emit directly to OpenTelemetry
 * span events (no parallel Smoo-native batched fetch). Browser keeps the
 * transport path because OTel browser SDK is too heavy for customer-facing
 * sites.
 */
export type CaptureHandler = (event: ObservabilityEvent, raw: { error?: unknown; message?: string; extra?: { tags?: Record<string, string> } }) => void;

/**
 * Singleton client used by both browser and Node entry points. The
 * transport (browser) or native capture handler (node) is wired in by the
 * runtime-specific entry (`src/browser/index.ts`, `src/node/index.ts`).
 */
class _Client {
    private options: ClientOptions | null = null;
    private runtime: Runtime = typeof window === 'undefined' ? 'node' : 'browser';
    private transport: ((batch: ObservabilityEvent[]) => Promise<void>) | null = null;
    private captureHandler: CaptureHandler | null = null;

    init(options: ClientOptions): void {
        this.options = options;
        // Wiring (transport for browser, OTel-native capture for node) happens
        // in the runtime-specific entry's init wrapper.
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

    /**
     * Register a runtime-native capture path. When set, captureException /
     * captureMessage route through this handler INSTEAD of the HTTP transport
     * — node uses this to write directly to OpenTelemetry span events so the
     * Smoo SDK speaks OTel natively. Calling with `null` un-registers.
     */
    _registerCaptureHandler(handler: CaptureHandler | null): void {
        this.captureHandler = handler;
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
        if (!final) return eventId;
        if (this.captureHandler) {
            try {
                this.captureHandler(final, { error, extra });
            } catch {
                /* swallow — observability must not throw */
            }
            return eventId;
        }
        if (this.transport) {
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
        if (!final) return eventId;
        if (this.captureHandler) {
            try {
                this.captureHandler(final, { message });
            } catch {
                /* swallow */
            }
            return eventId;
        }
        if (this.transport) {
            void this.transport([final]).catch(() => {});
        }
        return eventId;
    }
}

function toException(err: unknown): ExceptionInfo {
    if (err instanceof Error) {
        const exc: ExceptionInfo = {
            type: err.name,
            value: err.message,
            stacktrace: { frames: dropSdkFrames(parseStack(err.stack)) },
        };
        // Walk Error.cause for chained exceptions.
        const cause = (err as { cause?: unknown }).cause;
        if (cause !== undefined && cause !== null) {
            exc.cause = toException(cause);
        }
        return exc;
    }
    return {
        type: 'Unknown',
        value: typeof err === 'string' ? err : safeStringify(err),
        stacktrace: { frames: [] },
    };
}

function safeStringify(v: unknown): string {
    try {
        return JSON.stringify(v);
    } catch {
        return String(v);
    }
}

export const Client = new _Client();
export type { _Client };
