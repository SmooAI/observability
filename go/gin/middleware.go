// Package ginobs provides a Gin (github.com/gin-gonic/gin) middleware for the
// Smoo observability SDK. It is a thin adapter over the same Scope +
// CaptureException primitives as the core net/http middleware, with identical
// semantics:
//
//  1. Establishes a fresh request-scoped Scope on the request context
//     (c.Request = c.Request.WithContext(...)), so any CaptureExceptionOnSpan /
//     scope mutation fired from a downstream handler picks up this request's
//     identity.
//  2. Resolves user identity (via ResolveUser) and records a "request" context
//     block (method, path, allowlisted headers). Hydration is wrapped so a
//     failure there never breaks the request.
//  3. Captures downstream panics AND errors attached to the Gin context
//     (c.Errors) as exceptions (tagged source "gin.middleware"). Panics are
//     re-panicked by default so Gin's own Recovery middleware still runs; set
//     SwallowPanics to abort with a 500 instead.
//
// Lives in its own Go module so the core SDK does not take a hard dependency on
// Gin unless this adapter is imported.
package ginobs

import (
	"fmt"
	"net/http"

	"github.com/gin-gonic/gin"

	obs "github.com/SmooAI/observability/go"
)

// Options configures Middleware.
type Options struct {
	// Client to capture on. Defaults to the package obs.Default client.
	Client *obs.Client
	// ResolveUser extracts user identity from the request. Return nil to skip.
	ResolveUser func(c *gin.Context) *obs.User
	// RequestHeaderAllowlist names headers recorded on the request context.
	// Defaults to a conservative, safe-to-send set.
	RequestHeaderAllowlist []string
	// SwallowPanics, when true, makes the middleware capture a downstream panic,
	// abort with a 500, and NOT re-panic. The default (false) re-panics after
	// capturing so the host's own Recovery middleware still runs.
	SwallowPanics bool
}

var defaultHeaderAllowlist = []string{"user-agent", "referer", "x-request-id", "x-trace-id", "x-correlation-id"}

// Middleware returns a Gin handler that establishes per-request scope and
// captures downstream panics/errors.
func Middleware(opts Options) gin.HandlerFunc {
	client := opts.Client
	if client == nil {
		client = obs.Default
	}
	allowlist := opts.RequestHeaderAllowlist
	if allowlist == nil {
		allowlist = defaultHeaderAllowlist
	}
	rethrow := !opts.SwallowPanics

	return func(c *gin.Context) {
		if !client.IsInitialized() {
			c.Next()
			return
		}

		scope := obs.NewScope()
		ctx := obs.ContextWithScope(c.Request.Context(), scope)
		c.Request = c.Request.WithContext(ctx)

		hydrateScope(c, scope, opts.ResolveUser, allowlist)

		defer func() {
			if rec := recover(); rec != nil {
				client.CaptureExceptionOnSpan(ctx, panicToError(rec), map[string]string{"source": "gin.middleware"})
				if rethrow {
					panic(rec)
				}
				c.AbortWithStatus(http.StatusInternalServerError)
			}
		}()

		c.Next()

		// Capture any errors handlers attached to the Gin context (the idiomatic
		// way Gin handlers report failures via c.Error(err)).
		for _, ginErr := range c.Errors {
			client.CaptureExceptionOnSpan(ctx, ginErr.Err, map[string]string{"source": "gin.middleware"})
		}
	}
}

// New is the convenience constructor — captures panics then re-panics so the
// host's Recovery still runs.
func New(client *obs.Client, resolveUser func(c *gin.Context) *obs.User) gin.HandlerFunc {
	return Middleware(Options{Client: client, ResolveUser: resolveUser})
}

// hydrateScope sets user + request context on the scope. Wrapped so a panic in
// ResolveUser or header reads never breaks the request, mirroring the core
// middleware's recoverSilently guard.
func hydrateScope(c *gin.Context, scope *obs.Scope, resolveUser func(c *gin.Context) *obs.User, allowlist []string) {
	defer func() { _ = recover() }()
	if resolveUser != nil {
		if u := resolveUser(c); u != nil {
			scope.SetUser(u)
		}
	}
	headers := map[string]string{}
	for _, name := range allowlist {
		if v := c.GetHeader(name); v != "" {
			headers[name] = v
		}
	}
	scope.SetContext("request", map[string]any{
		"method":  c.Request.Method,
		"path":    c.FullPath(),
		"headers": headers,
	})
}

func panicToError(rec any) error {
	if err, ok := rec.(error); ok {
		return err
	}
	return fmt.Errorf("panic: %v", rec)
}
