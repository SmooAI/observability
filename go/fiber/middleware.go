// Package fiberobs provides a Fiber (github.com/gofiber/fiber/v2) middleware for
// the Smoo observability SDK. It is a thin adapter over the same Scope +
// CaptureException primitives as the core net/http middleware, with identical
// semantics:
//
//  1. Establishes a fresh request-scoped Scope on the request's user context
//     (Fiber's c.UserContext()), so any CaptureExceptionOnSpan / scope mutation
//     fired from a downstream handler picks up this request's identity.
//  2. Resolves user identity (via ResolveUser) and records a "request" context
//     block (method, path, allowlisted headers). Hydration is wrapped so a
//     failure there never breaks the request.
//  3. Captures downstream panics AND returned errors as exceptions (tagged
//     source "fiber.middleware") before propagating them. Panics are re-panicked
//     by default so Fiber's own recovery still runs; set SwallowPanics to write a
//     500 instead. Returned errors are always re-returned so Fiber's ErrorHandler
//     renders the response.
//
// Lives in its own Go module so the core SDK does not take a hard dependency on
// Fiber unless this adapter is imported.
package fiberobs

import (
	"fmt"
	"net/http"

	"github.com/gofiber/fiber/v2"

	obs "github.com/SmooAI/observability/go"
)

// Options configures Middleware.
type Options struct {
	// Client to capture on. Defaults to the package obs.Default client.
	Client *obs.Client
	// ResolveUser extracts user identity from the request. Return nil to skip.
	ResolveUser func(c *fiber.Ctx) *obs.User
	// RequestHeaderAllowlist names headers recorded on the request context.
	// Defaults to a conservative, safe-to-send set.
	RequestHeaderAllowlist []string
	// SwallowPanics, when true, makes the middleware capture a downstream panic,
	// write a 500, and NOT re-panic. The default (false) re-panics after
	// capturing so the host's own recovery middleware still runs.
	SwallowPanics bool
}

var defaultHeaderAllowlist = []string{"user-agent", "referer", "x-request-id", "x-trace-id", "x-correlation-id"}

// Middleware returns a Fiber handler that establishes per-request scope and
// captures downstream panics/errors.
func Middleware(opts Options) fiber.Handler {
	client := opts.Client
	if client == nil {
		client = obs.Default
	}
	allowlist := opts.RequestHeaderAllowlist
	if allowlist == nil {
		allowlist = defaultHeaderAllowlist
	}
	rethrow := !opts.SwallowPanics

	return func(c *fiber.Ctx) (err error) {
		if !client.IsInitialized() {
			return c.Next()
		}

		scope := obs.NewScope()
		ctx := obs.ContextWithScope(c.UserContext(), scope)
		c.SetUserContext(ctx)

		hydrateScope(c, scope, opts.ResolveUser, allowlist)

		defer func() {
			if rec := recover(); rec != nil {
				client.CaptureExceptionOnSpan(ctx, panicToError(rec), map[string]string{"source": "fiber.middleware"})
				if rethrow {
					panic(rec)
				}
				err = c.SendStatus(http.StatusInternalServerError)
			}
		}()

		err = c.Next()
		if err != nil {
			client.CaptureExceptionOnSpan(ctx, err, map[string]string{"source": "fiber.middleware"})
		}
		return err
	}
}

// New is the convenience constructor — captures panics then re-panics so the
// host's recovery still runs.
func New(client *obs.Client, resolveUser func(c *fiber.Ctx) *obs.User) fiber.Handler {
	return Middleware(Options{Client: client, ResolveUser: resolveUser})
}

// hydrateScope sets user + request context on the scope. Wrapped so a panic in
// ResolveUser or header reads never breaks the request, mirroring the core
// middleware's recoverSilently guard.
func hydrateScope(c *fiber.Ctx, scope *obs.Scope, resolveUser func(c *fiber.Ctx) *obs.User, allowlist []string) {
	defer func() { _ = recover() }()
	if resolveUser != nil {
		if u := resolveUser(c); u != nil {
			scope.SetUser(u)
		}
	}
	headers := map[string]string{}
	for _, name := range allowlist {
		if v := c.Get(name); v != "" {
			headers[name] = v
		}
	}
	scope.SetContext("request", map[string]any{
		"method":  c.Method(),
		"path":    c.Path(),
		"headers": headers,
	})
}

func panicToError(rec any) error {
	if err, ok := rec.(error); ok {
		return err
	}
	return fmt.Errorf("panic: %v", rec)
}
