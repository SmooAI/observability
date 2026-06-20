package observability

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"sync"
	"time"
)

// Batched HTTP transport. Holds a bounded queue, flushes on a timer or when
// MaxBatchSize events are buffered, and retries a failed batch by pushing it
// back to the front of the queue. Errors are swallowed — observability must
// never throw into host code. Mirrors the TS Transport.

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
	client        *http.Client

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
	// HTTPClient overrides the default client (test seam).
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
	client := opts.HTTPClient
	if client == nil {
		client = &http.Client{Timeout: 10 * time.Second}
	}
	return &Transport{
		dsn:           opts.DSN,
		flushInterval: flush,
		maxBatch:      batch,
		maxQueue:      queue,
		client:        client,
	}
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
	body, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, t.dsn, bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("content-type", "application/json")
	resp, err := t.client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
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
