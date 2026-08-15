package observability

import (
	"bytes"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// Crash-reporting tests. A panic that is reported and then re-panicked KILLS
// the process, so the only honest way to test it is to let a real process die:
// each case re-runs this test binary with OBS_CRASH_CHILD set, points the
// child's exporters at an httptest server in the parent, and then asserts on
// BOTH what the parent received and how the child died (exit status + stderr).
// Calling RecoverAndReport in-process would prove nothing about the two things
// that actually matter — that the flush beat the death, and that the death
// still happened with its original traceback.

const (
	crashChildEnv = "OBS_CRASH_CHILD"
	crashURLEnv   = "OBS_CRASH_URL"
	// Sentinel appears in the panic value, so it must show up both on the wire
	// and in the child's own crash output.
	crashSentinel = "boom-8f3a1c"
	// Distinct sentinel for a failure raised INSIDE the reporter — it must
	// never reach the child's crash output, because that would mean the
	// reporter's panic replaced the original one.
	reporterSentinel = "reporter-exploded-4d2e"
)

// TestCrashChild is the subprocess entry point. It is a no-op (skipped) in a
// normal `go test` run and only does anything when the parent sets the env
// guard.
func TestCrashChild(t *testing.T) {
	mode := os.Getenv(crashChildEnv)
	if mode == "" {
		t.Skip("subprocess entry point; driven by the TestRecoverAndReport* parents")
	}
	runCrashChild(mode)
	t.Fatalf("child mode %q was supposed to crash and did not", mode)
}

func runCrashChild(mode string) {
	url := os.Getenv(crashURLEnv)
	ctx := context.Background()
	Bootstrap(ctx, &BootstrapEnv{
		Endpoint:    url,
		DSN:         url + "/webhook",
		ServiceName: "crash-test",
		Environment: "test",
		Release:     "test",
	})

	if mode == "brokenreporter" {
		// A reporter-internal failure: BeforeSend runs on the crash path.
		opts := *Default.Options()
		opts.BeforeSend = func(ObservabilityEvent) *ObservabilityEvent {
			panic(reporterSentinel)
		}
		Default.Init(opts)
	}

	switch mode {
	case "nilctx":
		// A caller that never had a context. The reporter must still work.
		defer RecoverAndReport(nil) //nolint:staticcheck // deliberately nil
		crashBoom()
	case "cancelledctx":
		// The realistic worker shape: the goroutine's context is already
		// cancelled (shutdown in progress) when it panics.
		cctx, cancel := context.WithCancel(ctx)
		cancel()
		defer RecoverAndReport(cctx)
		crashBoom()
	}

	defer RecoverAndReport(ctx)

	switch mode {
	case "nilpanic":
		panic(nil)
	case "hostile":
		panic(hostilePanicValue{})
	case "hostileonce":
		panic(hostileOnceValue{})
	case "nested":
		nestedCrash(ctx)
	default: // "panic", "hang", "brokenreporter"
		crashBoom()
	}
}

// crashBoom is the named frame the parent looks for in the child's traceback.
func crashBoom() { panic(crashSentinel) }

// nestedCrash defers a second RecoverAndReport under the outer one.
func nestedCrash(ctx context.Context) {
	defer RecoverAndReport(ctx)
	crashBoom()
}

// hostilePanicValue is an error whose Error() panics — the nastiest value the
// reporter can be handed, since every reporting path stringifies it.
type hostilePanicValue struct{}

func (hostilePanicValue) Error() string { panic(reporterSentinel) }

// hostileOnceValue blows up only the FIRST time it is stringified, so the crash
// path has to survive a mid-report failure and still deliver the rest.
type hostileOnceValue struct{}

var hostileCalls atomic.Int32

func (hostileOnceValue) Error() string {
	if hostileCalls.Add(1) == 1 {
		panic(reporterSentinel)
	}
	return "hostile-once " + crashSentinel
}

// --- harness ---

type collector struct {
	mu   sync.Mutex
	hits map[string][][]byte
	srv  *httptest.Server
}

func newCollector(t *testing.T) *collector {
	t.Helper()
	c := &collector{hits: map[string][][]byte{}}
	c.srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		c.mu.Lock()
		c.hits[r.URL.Path] = append(c.hits[r.URL.Path], body)
		c.mu.Unlock()
		w.WriteHeader(http.StatusOK)
	}))
	t.Cleanup(c.srv.Close)
	return c
}

// bodies returns everything received on path.
func (c *collector) bodies(path string) [][]byte {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([][]byte(nil), c.hits[path]...)
}

// sawSentinel reports whether any body on path contained the sentinel.
func (c *collector) sawSentinel(path, sentinel string) bool {
	for _, b := range c.bodies(path) {
		if bytes.Contains(b, []byte(sentinel)) {
			return true
		}
	}
	return false
}

type childResult struct {
	exitCode int
	stderr   string
	elapsed  time.Duration
}

func runCrashSubprocess(t *testing.T, mode, url string) childResult {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=TestCrashChild", "-test.timeout=60s")
	cmd.Env = append(os.Environ(), crashChildEnv+"="+mode, crashURLEnv+"="+url)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	cmd.Stdout = io.Discard

	start := time.Now()
	err := cmd.Run()
	elapsed := time.Since(start)

	code := 0
	if ee, ok := err.(*exec.ExitError); ok {
		code = ee.ExitCode()
	} else if err != nil {
		t.Fatalf("running child %q: %v", mode, err)
	}
	return childResult{exitCode: code, stderr: stderr.String(), elapsed: elapsed}
}

// --- tests ---

// The headline case: the crash is reported on all three wires AND the process
// still dies exactly as it would have without us.
func TestRecoverAndReportReportsThenStillCrashes(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "panic", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0; a reported panic must still kill the process (stderr: %s)", res.stderr)
	}
	if !strings.Contains(res.stderr, "panic: "+crashSentinel) {
		t.Errorf("original panic missing from child stderr:\n%s", res.stderr)
	}
	// Go annotates a re-panicked panic "[recovered]" (<=1.25) /
	// "[recovered, repanicked]" (1.26+); match the stable substring.
	if !strings.Contains(res.stderr, "recovered") {
		t.Errorf("expected the recovered-then-repanicked marker in stderr:\n%s", res.stderr)
	}
	if !strings.Contains(res.stderr, "crashBoom") {
		t.Errorf("original panicking frame missing from the traceback:\n%s", res.stderr)
	}

	// Flush-before-death: every one of these arrives only because the crash
	// path force-flushed. All three are batched (1s transport timer, 5s span
	// batcher, 1s log batcher) and the child lives well under that.
	if !c.sawSentinel("/webhook", crashSentinel) {
		t.Errorf("webhook transport never delivered the crash event (got %d POSTs)", len(c.bodies("/webhook")))
	}
	if !c.sawSentinel("/v1/traces", crashSentinel) {
		t.Errorf("no exception span exported (got %d POSTs)", len(c.bodies("/v1/traces")))
	}
	if !c.sawSentinel("/v1/logs", crashSentinel) {
		t.Errorf("no FATAL log record exported (got %d POSTs)", len(c.bodies("/v1/logs")))
	}
}

// panic(nil) — since Go 1.21 the runtime substitutes *runtime.PanicNilError.
// The reporter must handle it and the process must still die.
func TestRecoverAndReportHandlesNilPanicValue(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "nilpanic", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 on panic(nil) (stderr: %s)", res.stderr)
	}
	if !strings.Contains(res.stderr, "nil") {
		t.Errorf("expected the nil-panic message in stderr:\n%s", res.stderr)
	}
	if len(c.bodies("/webhook")) == 0 {
		t.Errorf("nil panic was not reported at all")
	}
}

// An error value whose Error() panics: the reporter stringifies it on every
// path, so this is the value most likely to blow up the crash path.
func TestRecoverAndReportSurvivesHostilePanicValue(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "hostile", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 on a hostile panic value (stderr: %s)", res.stderr)
	}
	// The runtime itself fails to print the value, and says so — proof the
	// ORIGINAL value reached the runtime's crash printer, i.e. our reporter
	// neither swallowed it nor replaced it. Wording differs by Go version
	// ("PANIC=Error method" <=1.25, "panic while printing panic value" 1.26+).
	if !strings.Contains(res.stderr, "PANIC=Error method") &&
		!strings.Contains(res.stderr, "panic while printing panic value") {
		t.Errorf("expected the runtime's own failed-to-print-panic annotation:\n%s", res.stderr)
	}
	// The original panicking frame is still on the traceback. (Deliberately not
	// asserting on a RecoverAndReport frame: it is small enough that the
	// compiler inlines it into the defer wrapper on some builds.)
	if !strings.Contains(res.stderr, "runCrashChild") {
		t.Errorf("original panicking frame missing from the traceback:\n%s", res.stderr)
	}
	if strings.Contains(res.stderr, "observability.reportCrash") {
		t.Errorf("the reporter panicked out into the crash output:\n%s", res.stderr)
	}
}

// A failure raised inside the reporter must not suppress or replace the panic.
func TestRecoverAndReportSurvivesBrokenReporter(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "brokenreporter", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 when the reporter failed (stderr: %s)", res.stderr)
	}
	if !strings.Contains(res.stderr, "panic: "+crashSentinel) {
		t.Errorf("original panic lost when the reporter failed:\n%s", res.stderr)
	}
	if strings.Contains(res.stderr, reporterSentinel) {
		t.Errorf("the reporter's own panic replaced the original:\n%s", res.stderr)
	}
}

// Two RecoverAndReport defers on one goroutine: the outer one sees the inner
// one's re-panic. Double-reporting is documented; what must NOT happen is a
// changed panic value, a hang, or a swallowed crash.
func TestRecoverAndReportNestedDefersStillCrash(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "nested", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 with nested defers (stderr: %s)", res.stderr)
	}
	if !strings.Contains(res.stderr, "panic: "+crashSentinel) {
		t.Errorf("nested re-panic changed the panic value:\n%s", res.stderr)
	}
	if len(c.bodies("/webhook")) == 0 {
		t.Errorf("nested crash was not reported")
	}
}

// A wedged collector must not hold a dying process open: the flush budget is
// CrashFlushTimeout, shared across transport + OTel.
func TestCrashFlushIsBounded(t *testing.T) {
	block := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		select {
		case <-block:
		case <-r.Context().Done():
		}
	}))
	// Cleanups run LIFO: release the blocked handlers BEFORE srv.Close, which
	// waits for outstanding requests to return.
	t.Cleanup(srv.Close)
	t.Cleanup(func() { close(block) })

	res := runCrashSubprocess(t, "hang", srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 against a hung collector (stderr: %s)", res.stderr)
	}
	if !strings.Contains(res.stderr, "panic: "+crashSentinel) {
		t.Errorf("panic lost against a hung collector:\n%s", res.stderr)
	}
	// Unbounded, the OTLP exporter alone retries for ~1 minute. 10s leaves room
	// for process start-up on a loaded machine while still failing loudly if
	// the budget is not enforced.
	if res.elapsed > 10*time.Second {
		t.Errorf("child took %s to die; the crash flush is not bounded", res.elapsed)
	}
}

// A failure PART WAY THROUGH the report must not stop the remaining steps: the
// first stringify panics, the capture + flush after it must still land.
func TestRecoverAndReportContinuesAfterAMidReportFailure(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "hostileonce", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 (stderr: %s)", res.stderr)
	}
	if !strings.Contains(res.stderr, "panic: hostile-once") {
		t.Errorf("original panic missing from child stderr:\n%s", res.stderr)
	}
	if strings.Contains(res.stderr, reporterSentinel) {
		t.Errorf("the reporter's own panic escaped into the crash output:\n%s", res.stderr)
	}
	if !c.sawSentinel("/webhook", crashSentinel) {
		t.Errorf("report abandoned after the first failing step (got %d POSTs)", len(c.bodies("/webhook")))
	}
}

// A goroutine whose context is already cancelled still gets its crash out: the
// flush deliberately detaches from the caller's cancellation.
func TestRecoverAndReportFlushesWithACancelledContext(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "cancelledctx", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 (stderr: %s)", res.stderr)
	}
	if !c.sawSentinel("/webhook", crashSentinel) {
		t.Errorf("cancelled context suppressed the crash report (got %d POSTs)", len(c.bodies("/webhook")))
	}
}

// RecoverAndReport(nil) must report rather than trip over the nil context.
func TestRecoverAndReportWithNilContext(t *testing.T) {
	c := newCollector(t)
	res := runCrashSubprocess(t, "nilctx", c.srv.URL)

	if res.exitCode == 0 {
		t.Fatalf("child exited 0 (stderr: %s)", res.stderr)
	}
	if !c.sawSentinel("/webhook", crashSentinel) {
		t.Errorf("nil context suppressed the crash report (got %d POSTs)", len(c.bodies("/webhook")))
	}
}

// Deferred on a function that never panics, it must do nothing at all.
func TestRecoverAndReportWithoutPanicIsNoop(t *testing.T) {
	called := false
	func() {
		defer RecoverAndReport(context.Background())
		called = true
	}()
	if !called {
		t.Fatalf("unreachable")
	}
}
