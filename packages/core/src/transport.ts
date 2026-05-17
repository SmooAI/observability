import type { ClientOptions, IngestPayload, ObservabilityEvent } from './types';

const DEFAULT_FLUSH_MS = 1000;
const DEFAULT_BATCH_SIZE = 30;
const DEFAULT_QUEUE_MAX = 250;

interface TransportRuntimeAdapter {
    /** Whether `navigator.sendBeacon` is available (browser). */
    canBeacon: boolean;
    /** Beacon implementation, if available. */
    beacon?: (url: string, body: string) => boolean;
    /** Bind `pagehide` so we can flush via beacon. */
    bindLifecycle?: (onPageHide: () => void) => void;
}

/**
 * Universal batched transport. Holds a small queue, flushes on a timer or when
 * `maxBatchSize` events are buffered, and falls back to `sendBeacon` when the
 * page is unloading.
 *
 * Errors are swallowed — observability must never throw into user code.
 */
export class Transport {
    private queue: ObservabilityEvent[] = [];
    private timer: ReturnType<typeof setTimeout> | null = null;
    private inFlight = false;

    constructor(
        private readonly opts: Required<Pick<ClientOptions, 'dsn'>> & Pick<ClientOptions, 'flushIntervalMs' | 'maxBatchSize' | 'maxQueueSize'>,
        private readonly adapter: TransportRuntimeAdapter,
    ) {
        adapter.bindLifecycle?.(() => this.flushBeacon());
    }

    enqueue(event: ObservabilityEvent): void {
        const max = this.opts.maxQueueSize ?? DEFAULT_QUEUE_MAX;
        if (this.queue.length >= max) {
            // Drop oldest to make room — recent events are more useful.
            this.queue.shift();
        }
        this.queue.push(event);
        if (this.queue.length >= (this.opts.maxBatchSize ?? DEFAULT_BATCH_SIZE)) {
            void this.flush();
        } else if (!this.timer) {
            this.timer = setTimeout(() => void this.flush(), this.opts.flushIntervalMs ?? DEFAULT_FLUSH_MS);
        }
    }

    async flush(): Promise<void> {
        if (this.inFlight || this.queue.length === 0) {
            this.clearTimer();
            return;
        }
        this.inFlight = true;
        const batch = this.queue.splice(0, this.opts.maxBatchSize ?? DEFAULT_BATCH_SIZE);
        this.clearTimer();
        try {
            const payload: IngestPayload = { type: 'error', events: batch };
            await fetch(this.opts.dsn, {
                method: 'POST',
                headers: { 'content-type': 'application/json' },
                body: JSON.stringify(payload),
                keepalive: true,
            });
        } catch {
            // Best-effort: push events back to the front of the queue for next attempt.
            this.queue.unshift(...batch);
        } finally {
            this.inFlight = false;
            if (this.queue.length > 0 && !this.timer) {
                this.timer = setTimeout(() => void this.flush(), this.opts.flushIntervalMs ?? DEFAULT_FLUSH_MS);
            }
        }
    }

    flushBeacon(): void {
        if (this.queue.length === 0) return;
        if (!this.adapter.canBeacon || !this.adapter.beacon) {
            // Fall back to fire-and-forget fetch with keepalive.
            void this.flush();
            return;
        }
        const batch = this.queue.splice(0, this.queue.length);
        const payload: IngestPayload = { type: 'error', events: batch };
        const ok = this.adapter.beacon(this.opts.dsn, JSON.stringify(payload));
        if (!ok) {
            // Beacon failed (over 64KB or browser declined) — put events back.
            this.queue.unshift(...batch);
        }
    }

    private clearTimer(): void {
        if (this.timer) {
            clearTimeout(this.timer);
            this.timer = null;
        }
    }

    /** For tests. */
    _queueSize(): number {
        return this.queue.length;
    }
}
