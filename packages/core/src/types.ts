/**
 * @smooai/observability — public types.
 *
 * These mirror the Sentry "event envelope" shape closely enough that the
 * backend can fingerprint and store them without inventing a parallel schema,
 * while remaining first-class for Smoo (no Sentry dependency, no Sentry DSN).
 */

export type Level = 'fatal' | 'error' | 'warning' | 'info' | 'debug';
export type Runtime = 'browser' | 'node';

export interface ObservabilityEvent {
    /** Client-assigned event id (UUID v4). */
    eventId: string;
    /** When the event occurred, ms since epoch. */
    timestamp: number;
    /** Severity. Most captured exceptions are 'error'. */
    level: Level;
    /** Optional one-line message — for `captureMessage`. */
    message?: string;
    /** Exception chain (innermost first). */
    exception?: ExceptionInfo[];
    /** Breadcrumb buffer leading up to this event. */
    breadcrumbs?: Breadcrumb[];
    /** User context, if known. */
    user?: { id?: string; orgId?: string; sessionId?: string };
    /** Request context (browser) or Lambda invocation context (node). */
    request?: RequestInfo;
    /** Free-form tags for filtering in the dashboard. */
    tags?: Record<string, string>;
    /** Free-form contexts (e.g., browser, OS, device). */
    contexts?: Record<string, Record<string, unknown>>;
    /** Release identifier — git sha, Lambda version, etc. */
    release?: string;
    /** Deployment environment. */
    environment?: string;
    /** SDK self-identification. */
    sdk: { name: string; version: string; runtime: Runtime };
}

export interface ExceptionInfo {
    /** e.g., 'TypeError', 'ReferenceError', 'ChunkLoadError'. */
    type: string;
    /** Exception message. */
    value: string;
    /** Stack frames, innermost (most recent) first. */
    stacktrace: { frames: StackFrame[] };
    /** Linked cause (Error.cause chain), if any. */
    cause?: ExceptionInfo;
}

export interface StackFrame {
    /** Filename or module identifier (e.g., 'webpack-766ccbbf0ad1fc08.js'). */
    module: string;
    /** Function name from the stack. */
    function?: string;
    /** Line number in the original (pre-bundle) source if known. */
    lineno?: number;
    /** Column number. */
    colno?: number;
    /** True if the frame is application code (not node_modules / sdk-internal). */
    inApp?: boolean;
}

export interface Breadcrumb {
    timestamp: number;
    /** Free-form category — 'fetch', 'xhr', 'navigation', 'console', 'click', 'custom'. */
    category: string;
    /** 'info' for most, 'warning' / 'error' for failed fetches etc. */
    level: Level;
    /** Short human-readable summary. */
    message?: string;
    /** Free-form structured data. */
    data?: Record<string, unknown>;
}

export interface RequestInfo {
    /** Full URL (browser) or method + path (node). */
    url?: string;
    /** HTTP method. */
    method?: string;
    /** Selected headers (PII-scrubbed by default). */
    headers?: Record<string, string>;
    /** Query-string parameters. */
    queryString?: string;
    /** Currently we never include bodies — placeholder for opt-in. */
    body?: never;
}

/**
 * The transport envelope POSTed to the Smoo ingest endpoint. Discriminated
 * union with the existing `type: 'log'` path so one endpoint serves both.
 */
export interface IngestPayload {
    type: 'error';
    events: ObservabilityEvent[];
}

export interface ClientOptions {
    /** Ingest endpoint: `POST /webhooks/observability/{org_id}/{token}`. */
    dsn: string;
    /** Deployment environment string ('production', 'staging', ...). */
    environment?: string;
    /** Release id (git sha or Lambda version). */
    release?: string;
    /** Set false to skip auto-instrumentation of globals. */
    autoInstrumentation?: boolean;
    /** Max events kept in memory waiting to be flushed. */
    maxQueueSize?: number;
    /** Flush interval in ms (default 1000). */
    flushIntervalMs?: number;
    /** Max events per flush batch (default 30). */
    maxBatchSize?: number;
    /** Drop events that match this predicate (e.g., known noise). */
    beforeSend?: (event: ObservabilityEvent) => ObservabilityEvent | null;
}
