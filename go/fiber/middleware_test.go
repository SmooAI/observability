package fiberobs

import (
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gofiber/fiber/v2"
	fiberrecover "github.com/gofiber/fiber/v2/middleware/recover"

	obs "github.com/SmooAI/observability/go"
)

func TestMiddlewareSetsScopeAndPassesThrough(t *testing.T) {
	c := obs.NewClient()
	c.Init(obs.ClientOptions{DSN: "x"})

	var sawUser *obs.User
	app := fiber.New()
	app.Use(Middleware(Options{
		Client: c,
		ResolveUser: func(fc *fiber.Ctx) *obs.User {
			return &obs.User{ID: fc.Get("X-User")}
		},
	}))
	app.Get("/path", func(fc *fiber.Ctx) error {
		sawUser = obs.ScopeFromContext(fc.UserContext()).User()
		return fc.SendStatus(http.StatusOK)
	})

	req := httptest.NewRequest(http.MethodGet, "/path", nil)
	req.Header.Set("X-User", "u1")
	req.Header.Set("User-Agent", "test-agent")
	resp, err := app.Test(req)
	if err != nil {
		t.Fatalf("app.Test: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		t.Errorf("status = %d", resp.StatusCode)
	}
	if sawUser == nil || sawUser.ID != "u1" {
		t.Errorf("scope user not set: %+v", sawUser)
	}
}

func TestMiddlewareCapturesReturnedError(t *testing.T) {
	c := obs.NewClient()
	c.Init(obs.ClientOptions{DSN: "x"})
	var captured int
	c.RegisterTransport(func(b []obs.ObservabilityEvent) { captured += len(b) })

	app := fiber.New()
	app.Use(Middleware(Options{Client: c}))
	app.Get("/", func(fc *fiber.Ctx) error {
		return fiber.NewError(http.StatusBadGateway, "downstream boom")
	})

	resp, err := app.Test(httptest.NewRequest(http.MethodGet, "/", nil))
	if err != nil {
		t.Fatalf("app.Test: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusBadGateway {
		t.Errorf("status = %d, want 502 (error re-returned to Fiber ErrorHandler)", resp.StatusCode)
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

	// Pair with Fiber's recover middleware so the re-panic is turned into a 500
	// (Fiber, unlike Gin, has no recovery installed by default — panic recovery
	// is opt-in). The middleware re-panics after capturing (default), so Fiber's
	// recover renders the response.
	app := fiber.New()
	app.Use(fiberrecover.New())
	app.Use(Middleware(Options{Client: c})) // default: rethrow
	app.Get("/", func(fc *fiber.Ctx) error {
		panic("handler exploded")
	})

	resp, err := app.Test(httptest.NewRequest(http.MethodGet, "/", nil))
	if err != nil {
		t.Fatalf("app.Test: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusInternalServerError {
		t.Errorf("status = %d, want 500 (Fiber recovery after re-panic)", resp.StatusCode)
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

	app := fiber.New()
	app.Use(Middleware(Options{Client: c, SwallowPanics: true}))
	app.Get("/", func(fc *fiber.Ctx) error {
		panic("boom")
	})

	resp, err := app.Test(httptest.NewRequest(http.MethodGet, "/", nil))
	if err != nil {
		t.Fatalf("app.Test: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusInternalServerError {
		t.Errorf("status = %d, want 500", resp.StatusCode)
	}
	if captured != 1 {
		t.Errorf("expected 1 captured event, got %d", captured)
	}
}

func TestMiddlewarePassThroughWhenUninitialized(t *testing.T) {
	c := obs.NewClient() // not initialized
	called := false

	app := fiber.New()
	app.Use(Middleware(Options{Client: c}))
	app.Get("/", func(fc *fiber.Ctx) error {
		called = true
		// Scope helper still returns a non-nil scope even uninitialized.
		if obs.ScopeFromContext(fc.UserContext()) == nil {
			t.Error("nil scope")
		}
		return fc.SendStatus(http.StatusOK)
	})

	resp, err := app.Test(httptest.NewRequest(http.MethodGet, "/", nil))
	if err != nil {
		t.Fatalf("app.Test: %v", err)
	}
	defer func() { _, _ = io.Copy(io.Discard, resp.Body); resp.Body.Close() }()

	if !called {
		t.Error("handler not called")
	}
}
