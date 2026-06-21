package observability

import (
	"fmt"
	"net/http"
)

// net/http middleware — the Go framework integration, analogous to the Hono
// middleware in node/middleware.ts. For each request it:
//  1. Establishes a fresh request-scoped Scope on the request context.
//  2. Resolves user identity (via ResolveUser) and a "request" context block.
//  3. Recovers from panics in the downstream handler, capturing them as
//     exceptions (status 500) before re-panicking so the host's own recovery
//     can still run.
//
// Fiber and Gin integrations live in their own modules (go/fiber, go/gin) as
// thin adapters over the same Scope + CaptureException primitives. Echo is still
// deferred — see the gap note in the README.

// MiddlewareOptions configures Middleware.
type MiddlewareOptions struct {
	// Client to capture on. Defaults to the package Default client.
	Client *Client
	// ResolveUser extracts user identity from the request. Return nil to skip.
	ResolveUser func(r *http.Request) *User
	// RequestHeaderAllowlist names headers recorded on the request context.
	// Defaults to a conservative, safe-to-send set.
	RequestHeaderAllowlist []string
	// SwallowPanics, when true, makes the middleware capture a downstream
	// panic, write a 500, and NOT re-panic. The default (false) re-panics
	// after capturing so the host's own recovery middleware still runs.
	SwallowPanics bool
}

var defaultHeaderAllowlist = []string{"user-agent", "referer", "x-request-id", "x-trace-id", "x-correlation-id"}

// Middleware returns a net/http middleware (func(http.Handler) http.Handler).
func Middleware(opts MiddlewareOptions) func(http.Handler) http.Handler {
	client := opts.Client
	if client == nil {
		client = Default
	}
	allowlist := opts.RequestHeaderAllowlist
	if allowlist == nil {
		allowlist = defaultHeaderAllowlist
	}
	rethrow := !opts.SwallowPanics

	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if !client.IsInitialized() {
				next.ServeHTTP(w, r)
				return
			}

			scope := NewScope()
			ctx := ContextWithScope(r.Context(), scope)
			r = r.WithContext(ctx)

			func() {
				defer recoverSilently()
				if opts.ResolveUser != nil {
					if u := opts.ResolveUser(r); u != nil {
						scope.SetUser(u)
					}
				}
				headers := map[string]string{}
				for _, name := range allowlist {
					if v := r.Header.Get(name); v != "" {
						headers[name] = v
					}
				}
				scope.SetContext("request", map[string]any{
					"method":  r.Method,
					"path":    r.URL.Path,
					"headers": headers,
				})
			}()

			defer func() {
				if rec := recover(); rec != nil {
					err := panicToError(rec)
					client.CaptureExceptionOnSpan(ctx, err, map[string]string{"source": "http.middleware"})
					if rethrow {
						panic(rec)
					}
					w.WriteHeader(http.StatusInternalServerError)
				}
			}()

			next.ServeHTTP(w, r)
		})
	}
}

// NewMiddleware is the convenience constructor — captures panics then re-panics
// so the host's recovery still runs.
func NewMiddleware(client *Client, resolveUser func(r *http.Request) *User) func(http.Handler) http.Handler {
	return Middleware(MiddlewareOptions{
		Client:      client,
		ResolveUser: resolveUser,
	})
}

func panicToError(rec any) error {
	if err, ok := rec.(error); ok {
		return err
	}
	return fmt.Errorf("panic: %v", rec)
}
