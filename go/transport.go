package observability

import (
	"context"
	"errors"
	"net/http"
	"sync"
	"time"

	"github.com/SmooAI/fetch/go/fetch"
)

// Batched HTTP transport. Holds a bounded queue, flushes on a timer or when
// MaxBatchSize events are buffered, and retries a failed batch by pushing it
// back to the front of the queue. Errors are swallowed — observability must
// never throw into host code. Mirrors the TS Transport.
//
// Outbound delivery goes through github.com/SmooAI/fetch/go/fetch (SMOODEV-2026)
// so each batch POST gets the resilient stack — jittered exponential-backoff
// retries on 429/5xx + network errors, a per-request timeout, and an optional
// circuit breaker — instead of a bare net/http call. fetch handles transient
// blips within a single Flush; the transport's re-queue-on-failure handles
// longer outages across Flush cycles, so the two retry layers are complementary
// (fast in-call recovery vs. durable cross-cycle persistence), not duplicative:
// fetch aborts immediately on 4xx (RetryAbort), matching the old permanent-error
// behavior, so a persistent client error is not retried twice in a row.

const (
	defaultFlushMillis = 1000
	defaultBatchSize   = 30
	defaultQueueMax    = 250
)

// Transport is the batched event sender.
type Transport struct {
	dsn           string
	flushInterval time.Duration
	maxBatch      int
	maxQueue      int
	client        *fetch.Client

	mu       sync.Mutex
	queue    []ObservabilityEvent
	timer    *time.Timer
	inFlight bool
	closed   bool
}

// TransportOptions configures a Transport.
type TransportOptions struct {
	DSN           string
	FlushInterval time.Duration
	MaxBatchSize  int
	MaxQueueSize  int
	// HTTPClient overrides the underlying *http.Client that the resilient fetch
	// client drives (test seam). When nil, fetch's default transport is used.
	HTTPClient *http.Client
}

// NewTransport builds a Transport from options, applying defaults.
func NewTransport(opts TransportOptions) *Transport {
	flush := opts.FlushInterval
	if flush <= 0 {
		flush = defaultFlushMillis * time.Millisecond
	}
	batch := opts.MaxBatchSize
	if batch <= 0 {
		batch = defaultBatchSize
	}
	queue := opts.MaxQueueSize
	if queue <= 0 {
		queue = defaultQueueMax
	}
	return &Transport{
		dsn:           opts.DSN,
		flushInterval: flush,
		maxBatch:      batch,
		maxQueue:      queue,
		client:        buildFetchClient(opts.HTTPClient),
	}
}

// buildFetchClient assembles the resilient fetch client used for batch delivery.
// It keeps the prior 10s timeout and JSON content type, adds default retries
// (429/5xx + network, jittered backoff, aborts on 4xx), and lets a test seam
// swap the underlying *http.Client.
func buildFetchClient(httpClient *http.Client) *fetch.Client {
	retry := fetch.DefaultRetryOptions
	b := fetch.NewClientBuilder().
		WithTimeout(10 * time.Second).
		WithRetry(&retry).
		WithBaseHeaders(http.Header{"Content-Type": []string{"application/json"}})
	if httpClient != nil {
		b = b.WithHTTPClient(httpClient)
	}
	return b.Build()
}

// newTransportFromClientOptions wires a Transport from ClientOptions, used by
// the bootstrap / default init when a DSN is configured.
func newTransportFromClientOptions(opts ClientOptions) *Transport {
	flush := time.Duration(opts.FlushInterval) * time.Millisecond
	return NewTransport(TransportOptions{
		DSN:           opts.DSN,
		FlushInterval: flush,
		MaxBatchSize:  opts.MaxBatchSize,
		MaxQueueSize:  opts.MaxQueueSize,
	})
}

// Enqueue adds an event to the queue, dropping the oldest if the queue is full
// (recent events are more useful), and flushes immediately when a full batch is
// buffered.
func (t *Transport) Enqueue(event ObservabilityEvent) {
	defer recoverSilently()
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return
	}
	if len(t.queue) >= t.maxQueue {
		t.queue = t.queue[1:]
	}
	t.queue = append(t.queue, event)
	full := len(t.queue) >= t.maxBatch
	if !full && t.timer == nil {
		t.timer = time.AfterFunc(t.flushInterval, func() { _ = t.Flush(context.Background()) })
	}
	t.mu.Unlock()

	if full {
		_ = t.Flush(context.Background())
	}
}

// Flush sends one batch. Safe to call concurrently — a single batch is in
// flight at a time. Returns nil on success or when there's nothing to send.
func (t *Transport) Flush(ctx context.Context) error {
	t.mu.Lock()
	if t.inFlight || len(t.queue) == 0 {
		t.clearTimerLocked()
		t.mu.Unlock()
		return nil
	}
	t.inFlight = true
	n := t.maxBatch
	if n > len(t.queue) {
		n = len(t.queue)
	}
	batch := make([]ObservabilityEvent, n)
	copy(batch, t.queue[:n])
	t.queue = t.queue[n:]
	t.clearTimerLocked()
	t.mu.Unlock()

	err := t.send(ctx, batch)

	t.mu.Lock()
	t.inFlight = false
	if err != nil {
		// Best-effort: push the batch back to the front for the next attempt.
		t.queue = append(batch, t.queue...)
		if len(t.queue) > t.maxQueue {
			t.queue = t.queue[len(t.queue)-t.maxQueue:]
		}
	}
	if len(t.queue) > 0 && t.timer == nil && !t.closed {
		t.timer = time.AfterFunc(t.flushInterval, func() { _ = t.Flush(context.Background()) })
	}
	t.mu.Unlock()
	return err
}

func (t *Transport) send(ctx context.Context, batch []ObservabilityEvent) error {
	payload := IngestPayload{Type: "error", Events: batch}
	// fetch marshals the body to JSON and applies retries/timeout/backoff. It
	// returns a typed *fetch.HTTPResponseError for non-2xx responses after
	// retries are exhausted; normalize that to httpStatusError so callers and
	// tests see the same surface as before.
	resp, err := fetch.SimplePost(ctx, t.client, t.dsn, payload, nil)
	if err != nil {
		var httpErr *fetch.HTTPResponseError
		if errors.As(err, &httpErr) {
			return &httpStatusError{status: httpErr.StatusCode}
		}
		return err
	}
	if !resp.OK {
		return &httpStatusError{status: resp.StatusCode}
	}
	return nil
}

// Close stops the flush timer and prevents further enqueues. A final Flush
// should be called by the caller before Close if draining is desired.
func (t *Transport) Close() {
	t.mu.Lock()
	t.closed = true
	t.clearTimerLocked()
	t.mu.Unlock()
}

func (t *Transport) clearTimerLocked() {
	if t.timer != nil {
		t.timer.Stop()
		t.timer = nil
	}
}

// queueSize is a test seam.
func (t *Transport) queueSize() int {
	t.mu.Lock()
	defer t.mu.Unlock()
	return len(t.queue)
}

type httpStatusError struct{ status int }

func (e *httpStatusError) Error() string {
	return "observability: ingest returned HTTP " + http.StatusText(e.status)
}
