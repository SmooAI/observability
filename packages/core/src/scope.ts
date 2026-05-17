import type { Breadcrumb, Level, ObservabilityEvent } from './types';

export class Scope {
    private user: ObservabilityEvent['user'];
    private tags: Record<string, string> = {};
    private contexts: Record<string, Record<string, unknown>> = {};
    private breadcrumbs: Breadcrumb[] = [];
    private maxBreadcrumbs = 100;

    setUser(user: ObservabilityEvent['user']): void {
        this.user = user;
    }
    setTag(key: string, value: string): void {
        this.tags[key] = value;
    }
    setContext(key: string, ctx: Record<string, unknown>): void {
        this.contexts[key] = ctx;
    }
    addBreadcrumb(b: Breadcrumb): void {
        this.breadcrumbs.push(b);
        if (this.breadcrumbs.length > this.maxBreadcrumbs) {
            this.breadcrumbs.splice(0, this.breadcrumbs.length - this.maxBreadcrumbs);
        }
    }
    clearBreadcrumbs(): void {
        this.breadcrumbs.length = 0;
    }

    /** Merge this scope's state into an event. Called by `Client.captureException`. */
    applyToEvent(event: ObservabilityEvent): ObservabilityEvent {
        return {
            ...event,
            user: { ...this.user, ...event.user },
            tags: { ...this.tags, ...event.tags },
            contexts: { ...this.contexts, ...event.contexts },
            breadcrumbs: [...this.breadcrumbs, ...(event.breadcrumbs ?? [])],
        };
    }

    clone(): Scope {
        const s = new Scope();
        s.user = this.user;
        s.tags = { ...this.tags };
        s.contexts = { ...this.contexts };
        s.breadcrumbs = [...this.breadcrumbs];
        return s;
    }
}

const stack: Scope[] = [new Scope()];

export function getCurrentScope(): Scope {
    return stack[stack.length - 1]!;
}

export function withScope<T>(fn: (scope: Scope) => T): T {
    const next = getCurrentScope().clone();
    stack.push(next);
    try {
        return fn(next);
    } finally {
        stack.pop();
    }
}

export function _exposedStackForTests(): Scope[] {
    return stack;
}

export type { Level };
