package observability

import (
	"context"
	"testing"
)

func TestScopeApplyToEventMerges(t *testing.T) {
	s := NewScope()
	s.SetUser(&User{ID: "u1", OrgID: "org1"})
	s.SetTag("a", "1")
	s.SetContext("device", map[string]any{"os": "linux"})
	s.AddBreadcrumb(Breadcrumb{Category: "nav", Message: "home"})

	e := s.applyToEvent(ObservabilityEvent{
		Tags: map[string]string{"a": "override", "b": "2"},
		User: &User{SessionID: "sess1"},
	})

	if e.User.ID != "u1" || e.User.SessionID != "sess1" {
		t.Errorf("user merge wrong: %+v", e.User)
	}
	if e.Tags["a"] != "override" || e.Tags["b"] != "2" {
		t.Errorf("tag merge wrong: %+v", e.Tags)
	}
	if e.Contexts["device"]["os"] != "linux" {
		t.Errorf("context merge wrong: %+v", e.Contexts)
	}
	if len(e.Breadcrumbs) != 1 || e.Breadcrumbs[0].Category != "nav" {
		t.Errorf("breadcrumb merge wrong: %+v", e.Breadcrumbs)
	}
	if e.Breadcrumbs[0].Level != LevelInfo {
		t.Errorf("breadcrumb default level not applied: %q", e.Breadcrumbs[0].Level)
	}
}

func TestBreadcrumbCap(t *testing.T) {
	s := NewScope()
	for i := 0; i < 150; i++ {
		s.AddBreadcrumb(Breadcrumb{Category: "c", Message: "m"})
	}
	if len(s.breadcrumbs) != defaultMaxBreadcrumbs {
		t.Errorf("breadcrumb buffer = %d, want %d", len(s.breadcrumbs), defaultMaxBreadcrumbs)
	}
}

func TestWithScopeIsolation(t *testing.T) {
	ctx := context.Background()
	parent := NewScope()
	parent.SetTag("base", "yes")
	ctx = ContextWithScope(ctx, parent)

	WithScope(ctx, func(ctx context.Context, scope *Scope) {
		scope.SetTag("child", "only")
		got := ScopeFromContext(ctx)
		if got.tags["base"] != "yes" || got.tags["child"] != "only" {
			t.Errorf("child scope tags wrong: %+v", got.tags)
		}
	})

	// Parent must not have the child's tag.
	if _, ok := parent.tags["child"]; ok {
		t.Error("child tag leaked into parent scope")
	}
}

func TestScopeFromContextNeverNil(t *testing.T) {
	if ScopeFromContext(nil) == nil {
		t.Fatal("ScopeFromContext(nil) returned nil")
	}
	if ScopeFromContext(context.Background()) == nil {
		t.Fatal("ScopeFromContext(bg) returned nil")
	}
}

func TestContextMutators(t *testing.T) {
	s := NewScope()
	ctx := ContextWithScope(context.Background(), s)
	SetUser(ctx, &User{ID: "x"})
	SetTag(ctx, "k", "v")
	SetContext(ctx, "c", map[string]any{"y": 1})
	AddBreadcrumb(ctx, "cat", "msg", nil, "")

	if s.user.ID != "x" || s.tags["k"] != "v" || s.contexts["c"]["y"] != 1 {
		t.Errorf("context mutators did not write through: %+v", s)
	}
	if len(s.breadcrumbs) != 1 || s.breadcrumbs[0].Level != LevelInfo {
		t.Errorf("breadcrumb not added with default level: %+v", s.breadcrumbs)
	}
}
