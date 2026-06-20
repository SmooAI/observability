package observability

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestMiddlewareSetsScopeAndPassesThrough(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "x"})

	var sawUser *User
	mw := Middleware(MiddlewareOptions{
		Client: c,
		ResolveUser: func(r *http.Request) *User {
			return &User{ID: r.Header.Get("X-User")}
		},
	})

	handler := mw(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		sawUser = ScopeFromContext(r.Context()).user
		w.WriteHeader(http.StatusOK)
	}))

	req := httptest.NewRequest(http.MethodGet, "/path", nil)
	req.Header.Set("X-User", "u1")
	req.Header.Set("User-Agent", "test-agent")
	rr := httptest.NewRecorder()
	handler.ServeHTTP(rr, req)

	if rr.Code != http.StatusOK {
		t.Errorf("status = %d", rr.Code)
	}
	if sawUser == nil || sawUser.ID != "u1" {
		t.Errorf("scope user not set: %+v", sawUser)
	}
}

func TestMiddlewareCapturesPanicAndRethrows(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "x"})
	var captured int
	c.RegisterTransport(func(b []ObservabilityEvent) { captured += len(b) })

	mw := Middleware(MiddlewareOptions{Client: c}) // default: rethrow
	handler := mw(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		panic("handler exploded")
	}))

	defer func() {
		if rec := recover(); rec == nil {
			t.Error("expected re-panic")
		}
		if captured != 1 {
			t.Errorf("expected 1 captured event, got %d", captured)
		}
	}()
	handler.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodGet, "/", nil))
}

func TestMiddlewareSwallowPanics(t *testing.T) {
	c := NewClient()
	c.Init(ClientOptions{DSN: "x"})
	var captured int
	c.RegisterTransport(func(b []ObservabilityEvent) { captured += len(b) })

	mw := Middleware(MiddlewareOptions{Client: c, SwallowPanics: true})
	handler := mw(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		panic("boom")
	}))

	rr := httptest.NewRecorder()
	// Must not panic out.
	handler.ServeHTTP(rr, httptest.NewRequest(http.MethodGet, "/", nil))
	if rr.Code != http.StatusInternalServerError {
		t.Errorf("status = %d, want 500", rr.Code)
	}
	if captured != 1 {
		t.Errorf("expected 1 captured event, got %d", captured)
	}
}

func TestMiddlewarePassThroughWhenUninitialized(t *testing.T) {
	c := NewClient() // not initialized
	called := false
	mw := Middleware(MiddlewareOptions{Client: c})
	handler := mw(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		called = true
		// Scope still present even uninitialized.
		if ScopeFromContext(r.Context()) == nil {
			t.Error("nil scope")
		}
	}))
	handler.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodGet, "/", nil))
	if !called {
		t.Error("handler not called")
	}
}
