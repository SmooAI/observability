package ginobs

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"

	obs "github.com/SmooAI/observability/go"
)

func init() { gin.SetMode(gin.TestMode) }

func TestMiddlewareSetsScopeAndPassesThrough(t *testing.T) {
	c := obs.NewClient()
	c.Init(obs.ClientOptions{DSN: "x"})

	var sawUser *obs.User
	r := gin.New()
	r.Use(Middleware(Options{
		Client: c,
		ResolveUser: func(gc *gin.Context) *obs.User {
			return &obs.User{ID: gc.GetHeader("X-User")}
		},
	}))
	r.GET("/path", func(gc *gin.Context) {
		sawUser = obs.ScopeFromContext(gc.Request.Context()).User()
		gc.Status(http.StatusOK)
	})

	req := httptest.NewRequest(http.MethodGet, "/path", nil)
	req.Header.Set("X-User", "u1")
	req.Header.Set("User-Agent", "test-agent")
	rr := httptest.NewRecorder()
	r.ServeHTTP(rr, req)

	if rr.Code != http.StatusOK {
		t.Errorf("status = %d", rr.Code)
	}
	if sawUser == nil || sawUser.ID != "u1" {
		t.Errorf("scope user not set: %+v", sawUser)
	}
}

func TestMiddlewareCapturesGinContextErrors(t *testing.T) {
	c := obs.NewClient()
	c.Init(obs.ClientOptions{DSN: "x"})
	var captured int
	c.RegisterTransport(func(b []obs.ObservabilityEvent) { captured += len(b) })

	r := gin.New()
	r.Use(Middleware(Options{Client: c}))
	r.GET("/", func(gc *gin.Context) {
		// Idiomatic Gin error reporting.
		_ = gc.Error(errors.New("downstream boom"))
		gc.Status(http.StatusBadGateway)
	})

	rr := httptest.NewRecorder()
	r.ServeHTTP(rr, httptest.NewRequest(http.MethodGet, "/", nil))

	if rr.Code != http.StatusBadGateway {
		t.Errorf("status = %d, want 502", rr.Code)
	}
	if captured != 1 {
		t.Errorf("expected 1 captured event, got %d", captured)
	}
}

func TestMiddlewareCapturesPanicAndRethrows(t *testing.T) {
	c := obs.NewClient()
	c.Init(obs.ClientOptions{DSN: "x"})
	var captured int
	c.RegisterTransport(func(b []obs.ObservabilityEvent) { captured += len(b) })

	// Pair with Gin's Recovery so the re-panic is turned into a 500 response.
	r := gin.New()
	r.Use(gin.Recovery())
	r.Use(Middleware(Options{Client: c})) // default: rethrow
	r.GET("/", func(gc *gin.Context) {
		panic("handler exploded")
	})

	rr := httptest.NewRecorder()
	r.ServeHTTP(rr, httptest.NewRequest(http.MethodGet, "/", nil))

	if rr.Code != http.StatusInternalServerError {
		t.Errorf("status = %d, want 500 (Gin Recovery after re-panic)", rr.Code)
	}
	if captured != 1 {
		t.Errorf("expected 1 captured event, got %d", captured)
	}
}

func TestMiddlewareSwallowPanics(t *testing.T) {
	c := obs.NewClient()
	c.Init(obs.ClientOptions{DSN: "x"})
	var captured int
	c.RegisterTransport(func(b []obs.ObservabilityEvent) { captured += len(b) })

	// No gin.Recovery — SwallowPanics aborts with a 500 itself.
	r := gin.New()
	r.Use(Middleware(Options{Client: c, SwallowPanics: true}))
	r.GET("/", func(gc *gin.Context) {
		panic("boom")
	})

	rr := httptest.NewRecorder()
	r.ServeHTTP(rr, httptest.NewRequest(http.MethodGet, "/", nil))

	if rr.Code != http.StatusInternalServerError {
		t.Errorf("status = %d, want 500", rr.Code)
	}
	if captured != 1 {
		t.Errorf("expected 1 captured event, got %d", captured)
	}
}

func TestMiddlewarePassThroughWhenUninitialized(t *testing.T) {
	c := obs.NewClient() // not initialized
	called := false

	r := gin.New()
	r.Use(Middleware(Options{Client: c}))
	r.GET("/", func(gc *gin.Context) {
		called = true
		if obs.ScopeFromContext(gc.Request.Context()) == nil {
			t.Error("nil scope")
		}
		gc.Status(http.StatusOK)
	})

	rr := httptest.NewRecorder()
	r.ServeHTTP(rr, httptest.NewRequest(http.MethodGet, "/", nil))

	if !called {
		t.Error("handler not called")
	}
}
