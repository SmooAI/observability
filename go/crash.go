package observability

import (
	"context"
	"runtime/debug"
	"sync"
	"time"

	"go.opentelemetry.io/otel/log"
	logglobal "go.opentelemetry.io/otel/log/global"
)

// Unhandled-crash reporting — the Go counterpart of the Rust
// `install_panic_hook` (rust/observability/src/otel_capture.rs) and the TS
// `registerNodeGlobalHandlers` (packages/core/src/node/global-handlers.ts).
//
// # What Go can and cannot do — read this before trusting it
//
// Rust has `panic::set_hook`, Node has `process.on('uncaughtException')`,
// Python has `sys.excepthook`. GO HAS NO EQUIVALENT. An unrecovered panic in
// any goroutine runs that goroutine's deferred calls, prints the traceback and
// terminates the process; there is no process-wide interception point, and
// `recover` only works in a deferred function on the SAME goroutine as the
// panic. So this file cannot give you what Rust gets for free, and pretending
// otherwise would be worse than nothing: a crash reporter that quietly misses
// most crashes buys false confidence.
//
// COVERED:
//   - a goroutine whose entry point defers RecoverAndReport;
//   - panics inside HTTP handlers, already, via the net/http, Fiber and Gin
//     middlewares — those re-panic into the framework's recovery, so the
//     process survives and the normal batched export delivers the event.
//
// NOT COVERED (no mechanism exists):
//   - a panic in any goroutine the consumer spawned without the defer — that
//     is the case that kills the process, and it is on the caller to decorate
//     its `go` statements;
//   - runtime FATAL errors, which no defer can recover: "concurrent map
//     writes", "all goroutines are asleep - deadlock", stack exhaustion, OOM;
//   - os.Exit, SIGKILL, and death by signal.
//
// Rejected alternatives, so the next person does not re-litigate them:
//   - debug.SetCrashOutput (Go 1.23+) redirects the runtime's crash TEXT to a
//     second fd; nothing of ours runs afterwards. To act on it you need a
//     separate monitor process reading the pipe (the x/telemetry crashmonitor
//     pattern: re-exec yourself as a child), which an SDK cannot impose on its
//     host. Pointing it at a regular file and reporting on next boot fails our
//     actual deployment target: a restarted container gets a fresh writable
//     layer, so the file is gone precisely when we would want to read it.
//   - Wrapping the SDK's own timer goroutines: the two spawn points
//     (transport.go's time.AfterFunc) only call Transport.Flush, whose
//     dependencies are constructed unconditionally; a panic there would be an
//     SDK bug to fix, not a host crash to report.

// crashScopeName is the OTel instrumentation scope for crash log records.
const crashScopeName = "smooai.observability"

// CrashFlushTimeout bounds how long a dying process may wait for its crash
// report to reach the wire. 2s matches the TS SDK's lifecycle-flush budget and
// OtelSDKHandle.Flush's own default: one OTLP POST to an in-VPC collector is
// tens of milliseconds, so 2s covers a slow round trip plus a retry, while
// keeping a wedged collector from holding a crashing process open. The whole
// budget is shared across the webhook transport and the OTel providers — a slow
// first flush eats the second one's time rather than extending the total.
var CrashFlushTimeout = 2 * time.Second

// RecoverAndReport reports a panicking goroutine to observability and then
// RE-PANICS, so crash behaviour is unchanged: same panic value, same traceback
// (annotated "[recovered]"), same non-zero exit status.
//
// Use it as the first statement of every goroutine you spawn — including main:
//
//	func main() {
//	    ctx := context.Background()
//	    obs.Bootstrap(ctx, nil)
//	    defer obs.RecoverAndReport(ctx)
//	    ...
//	}
//
//	go func() {
//	    defer obs.RecoverAndReport(ctx)
//	    work()
//	}()
//
// It must be deferred directly (`defer obs.RecoverAndReport(ctx)`); wrapping it
// in another function breaks recover(), and it then does nothing. Called with
// no panic in flight it is a no-op, so it is safe on any function.
//
// Reports on the Default client (the one Bootstrap configures). Nested use
// double-reports — an inner and an outer deferred RecoverAndReport on the same
// goroutine each see the panic — because the re-panic must carry the original
// value untouched; de-duplication belongs server-side.
func RecoverAndReport(ctx context.Context) {
	rec := recover()
	if rec == nil {
		return
	}
	reportCrash(ctx, rec, "panic")
	panic(rec)
}

// reportCrash emits the crash as a FATAL log record and an exception on the
// active span, then flushes. Never panics: each step is individually guarded,
// because a panic in the crash path would replace the original panic and
// destroy the evidence we are here to preserve.
func reportCrash(ctx context.Context, rec any, source string) {
	defer recoverSilently()
	if ctx == nil {
		ctx = context.Background()
	}
	// Taken here, inside the deferred call, while the panicking frames are
	// still on the goroutine's stack.
	stack := string(debug.Stack())
	err := panicToError(rec)

	safe(func() { emitCrashLog(ctx, err, stack) })
	safe(func() {
		Default.CaptureExceptionOnSpan(ctx, err, map[string]string{"source": source, "mechanism": "panic"})
	})
	safe(func() { flushCrash(ctx) })
}

// emitCrashLog writes the crash as an OTel FATAL log record, correlated to the
// active span in ctx. Matches the Rust hook's ERROR/FATAL-level report; the
// exception EVENT + span status come from CaptureExceptionOnSpan.
func emitCrashLog(ctx context.Context, err error, stack string) {
	var r log.Record
	r.SetTimestamp(time.Now())
	r.SetSeverity(log.SeverityFatal)
	r.SetSeverityText("FATAL")
	r.SetBody(log.StringValue("panic: " + errString(err) + "\n" + stack))
	r.AddAttributes(
		log.String("exception.type", errorTypeName(err)),
		log.String("exception.message", errString(err)),
		log.String("exception.stacktrace", stack),
	)
	logglobal.GetLoggerProvider().Logger(crashScopeName).Emit(ctx, r)
}

// flushCrash drains the webhook transport and the OTel pipelines within one
// shared, bounded budget. Without it the report dies with the process: both
// paths are batched (1s transport timer, 5s span batcher).
func flushCrash(ctx context.Context) {
	defer recoverSilently()
	timeout := CrashFlushTimeout
	if timeout <= 0 {
		timeout = 2 * time.Second
	}
	// WithoutCancel: the crashing goroutine's ctx is often an already-cancelled
	// request context, which would abort the flush before it left the process.
	fctx, cancel := context.WithTimeout(context.WithoutCancel(ctx), timeout)
	defer cancel()

	// Transport first: it is one small JSON POST and it feeds the Errors
	// dashboard, the surface where a silently vanishing service is noticed.
	if t := activeCrashTransport(); t != nil {
		_ = t.Flush(fctx)
	}
	if h := installedOtel(); h != nil {
		h.Flush(fctx, timeout)
	}
}

// safe runs f, swallowing any panic, so one failing step cannot skip the rest
// of the crash path.
func safe(f func()) {
	defer recoverSilently()
	f()
}

var (
	crashTransportMu sync.Mutex
	crashTransportV  *Transport
)

// registerCrashTransport records the webhook transport so the crash path can
// flush it. Called by Bootstrap; a nil value un-registers.
func registerCrashTransport(t *Transport) {
	crashTransportMu.Lock()
	defer crashTransportMu.Unlock()
	crashTransportV = t
}

func activeCrashTransport() *Transport {
	crashTransportMu.Lock()
	defer crashTransportMu.Unlock()
	return crashTransportV
}

// installedOtel returns the handle SetupOtelSDK installed, whether it was
// called by Bootstrap or directly.
func installedOtel() *OtelSDKHandle {
	otelInstallMu.Lock()
	defer otelInstallMu.Unlock()
	return otelInstalled
}
