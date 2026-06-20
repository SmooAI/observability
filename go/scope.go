package observability

import (
	"context"
	"sync"
	"time"
)

// Scope carries the user / tags / contexts / breadcrumbs that are merged into an
// event at capture time. In the TS SDK the scope is a module-level stack; in Go
// we carry it on context.Context so it's request-safe in long-lived,
// concurrent servers (no AsyncLocalStorage caveat).
//
// A Scope is safe for concurrent use.
type Scope struct {
	mu             sync.Mutex
	user           *User
	tags           map[string]string
	contexts       map[string]map[string]any
	breadcrumbs    []Breadcrumb
	maxBreadcrumbs int
}

const defaultMaxBreadcrumbs = 100

// NewScope returns an empty scope with the default breadcrumb cap.
func NewScope() *Scope {
	return &Scope{
		tags:           map[string]string{},
		contexts:       map[string]map[string]any{},
		maxBreadcrumbs: defaultMaxBreadcrumbs,
	}
}

// SetUser sets the user context.
func (s *Scope) SetUser(u *User) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.user = u
}

// SetTag sets a single tag.
func (s *Scope) SetTag(key, value string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.tags == nil {
		s.tags = map[string]string{}
	}
	s.tags[key] = value
}

// SetContext sets a free-form structured context block.
func (s *Scope) SetContext(key string, ctx map[string]any) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.contexts == nil {
		s.contexts = map[string]map[string]any{}
	}
	s.contexts[key] = ctx
}

// AddBreadcrumb appends a breadcrumb, enforcing the max-100 ring (oldest dropped
// first), matching the TS behavior.
func (s *Scope) AddBreadcrumb(b Breadcrumb) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if b.Timestamp == 0 {
		b.Timestamp = nowMillis()
	}
	if b.Level == "" {
		b.Level = LevelInfo
	}
	s.breadcrumbs = append(s.breadcrumbs, b)
	if len(s.breadcrumbs) > s.maxBreadcrumbs {
		s.breadcrumbs = s.breadcrumbs[len(s.breadcrumbs)-s.maxBreadcrumbs:]
	}
}

// ClearBreadcrumbs empties the breadcrumb buffer.
func (s *Scope) ClearBreadcrumbs() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.breadcrumbs = nil
}

// applyToEvent merges this scope's state into an event. Event-level values win
// over scope-level values (matching the TS spread order).
func (s *Scope) applyToEvent(e ObservabilityEvent) ObservabilityEvent {
	s.mu.Lock()
	defer s.mu.Unlock()

	// User: scope value, then event value on top (per-field merge).
	merged := mergeUser(s.user, e.User)
	if merged != nil {
		e.User = merged
	}

	// Tags: scope base, event overrides.
	if len(s.tags) > 0 || len(e.Tags) > 0 {
		out := make(map[string]string, len(s.tags)+len(e.Tags))
		for k, v := range s.tags {
			out[k] = v
		}
		for k, v := range e.Tags {
			out[k] = v
		}
		e.Tags = out
	}

	// Contexts: scope base, event overrides.
	if len(s.contexts) > 0 || len(e.Contexts) > 0 {
		out := make(map[string]map[string]any, len(s.contexts)+len(e.Contexts))
		for k, v := range s.contexts {
			out[k] = v
		}
		for k, v := range e.Contexts {
			out[k] = v
		}
		e.Contexts = out
	}

	// Breadcrumbs: scope first, then event.
	if len(s.breadcrumbs) > 0 || len(e.Breadcrumbs) > 0 {
		out := make([]Breadcrumb, 0, len(s.breadcrumbs)+len(e.Breadcrumbs))
		out = append(out, s.breadcrumbs...)
		out = append(out, e.Breadcrumbs...)
		e.Breadcrumbs = out
	}

	return e
}

// clone returns a deep-ish copy used by WithScope so child mutations don't leak
// into the parent.
func (s *Scope) clone() *Scope {
	s.mu.Lock()
	defer s.mu.Unlock()
	c := NewScope()
	c.maxBreadcrumbs = s.maxBreadcrumbs
	if s.user != nil {
		u := *s.user
		c.user = &u
	}
	for k, v := range s.tags {
		c.tags[k] = v
	}
	for k, v := range s.contexts {
		c.contexts[k] = v
	}
	c.breadcrumbs = append(c.breadcrumbs, s.breadcrumbs...)
	return c
}

func mergeUser(base, override *User) *User {
	if base == nil && override == nil {
		return nil
	}
	out := User{}
	if base != nil {
		out = *base
	}
	if override != nil {
		if override.ID != "" {
			out.ID = override.ID
		}
		if override.OrgID != "" {
			out.OrgID = override.OrgID
		}
		if override.SessionID != "" {
			out.SessionID = override.SessionID
		}
	}
	return &out
}

// --- context.Context carriage ---

type scopeKey struct{}

// ScopeFromContext returns the scope stored on ctx, or a fresh empty scope if
// none is present. Never returns nil.
func ScopeFromContext(ctx context.Context) *Scope {
	if ctx != nil {
		if s, ok := ctx.Value(scopeKey{}).(*Scope); ok && s != nil {
			return s
		}
	}
	return NewScope()
}

// ContextWithScope returns a child context carrying the given scope.
func ContextWithScope(ctx context.Context, s *Scope) context.Context {
	if ctx == nil {
		ctx = context.Background()
	}
	return context.WithValue(ctx, scopeKey{}, s)
}

// WithScope clones the scope on ctx, passes the clone to fn, and runs fn with a
// context carrying that clone. Mutations on the clone do not affect the parent
// scope — mirrors the TS withScope isolation.
func WithScope(ctx context.Context, fn func(ctx context.Context, scope *Scope)) {
	defer recoverSilently()
	parent := ScopeFromContext(ctx)
	child := parent.clone()
	childCtx := ContextWithScope(ctx, child)
	fn(childCtx, child)
}

// --- convenience scope mutators bound to a context ---

// SetUser sets the user on the scope carried by ctx.
func SetUser(ctx context.Context, u *User) { ScopeFromContext(ctx).SetUser(u) }

// SetTag sets a tag on the scope carried by ctx.
func SetTag(ctx context.Context, key, value string) { ScopeFromContext(ctx).SetTag(key, value) }

// SetContext sets a context block on the scope carried by ctx.
func SetContext(ctx context.Context, key string, c map[string]any) {
	ScopeFromContext(ctx).SetContext(key, c)
}

// AddBreadcrumb adds a breadcrumb to the scope carried by ctx.
func AddBreadcrumb(ctx context.Context, category, message string, data map[string]any, level Level) {
	if level == "" {
		level = LevelInfo
	}
	ScopeFromContext(ctx).AddBreadcrumb(Breadcrumb{
		Category:  category,
		Message:   message,
		Data:      data,
		Level:     level,
		Timestamp: nowMillis(),
	})
}

func nowMillis() int64 {
	return time.Now().UnixMilli()
}
