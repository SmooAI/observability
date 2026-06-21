package observability

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestTransportFlushPostsBatch(t *testing.T) {
	var (
		mu       sync.Mutex
		received IngestPayload
		hits     int32
	)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&hits, 1)
		body, _ := io.ReadAll(r.Body)
		mu.Lock()
		_ = json.Unmarshal(body, &received)
		mu.Unlock()
		if r.Header.Get("content-type") != "application/json" {
			t.Errorf("content-type = %q", r.Header.Get("content-type"))
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	tr := NewTransport(TransportOptions{DSN: srv.URL, MaxBatchSize: 2})
	tr.Enqueue(ObservabilityEvent{EventID: "1"})
	if tr.queueSize() != 1 {
		t.Fatalf("queue should hold 1 before batch threshold, got %d", tr.queueSize())
	}
	tr.Enqueue(ObservabilityEvent{EventID: "2"}) // triggers immediate flush

	// Flush is synchronous in Enqueue's full path; give the goroutine-free path a beat anyway.
	waitFor(t, func() bool { return atomic.LoadInt32(&hits) == 1 })

	mu.Lock()
	defer mu.Unlock()
	if received.Type != "error" || len(received.Events) != 2 {
		t.Errorf("payload wrong: %+v", received)
	}
}

func TestTransportTimerFlush(t *testing.T) {
	var hits int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&hits, 1)
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	tr := NewTransport(TransportOptions{DSN: srv.URL, MaxBatchSize: 10, FlushInterval: 20 * time.Millisecond})
	tr.Enqueue(ObservabilityEvent{EventID: "1"})
	waitFor(t, func() bool { return atomic.LoadInt32(&hits) == 1 })
}

func TestTransportRetryOnFailure(t *testing.T) {
	var hits int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt32(&hits, 1)
		if n == 1 {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	tr := NewTransport(TransportOptions{DSN: srv.URL, MaxBatchSize: 1, FlushInterval: 10 * time.Millisecond})
	tr.Enqueue(ObservabilityEvent{EventID: "1"}) // first flush 500s, batch re-queued
	waitFor(t, func() bool { return atomic.LoadInt32(&hits) >= 2 })
	// After the successful retry, the queue should be empty.
	waitFor(t, func() bool { return tr.queueSize() == 0 })
}

func TestTransportQueueCap(t *testing.T) {
	tr := NewTransport(TransportOptions{DSN: "http://127.0.0.1:0", MaxBatchSize: 1000, MaxQueueSize: 3})
	tr.Close() // prevent flush; just test the ring
	tr2 := NewTransport(TransportOptions{DSN: "http://127.0.0.1:0", MaxBatchSize: 1000, MaxQueueSize: 3})
	for i := 0; i < 10; i++ {
		tr2.Enqueue(ObservabilityEvent{EventID: string(rune('a' + i))})
	}
	if tr2.queueSize() > 3 {
		t.Errorf("queue exceeded cap: %d", tr2.queueSize())
	}
}

func TestTransportClosedDropsEnqueue(t *testing.T) {
	tr := NewTransport(TransportOptions{DSN: "http://127.0.0.1:0"})
	tr.Close()
	tr.Enqueue(ObservabilityEvent{EventID: "x"})
	if tr.queueSize() != 0 {
		t.Errorf("closed transport accepted enqueue: %d", tr.queueSize())
	}
}

func TestFlushEmptyNoop(t *testing.T) {
	tr := NewTransport(TransportOptions{DSN: "http://127.0.0.1:0"})
	if err := tr.Flush(context.Background()); err != nil {
		t.Errorf("empty flush returned error: %v", err)
	}
}

// TestTransportFetchInCallRetry verifies the resilient fetch stack retries a
// transient 5xx within a single send/Flush (SMOODEV-2026) — the first attempt
// 500s, fetch backs off and retries, and the batch lands without ever needing
// the transport's slower re-queue path. queueSize stays 0 after the flush.
func TestTransportFetchInCallRetry(t *testing.T) {
	var hits int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if atomic.AddInt32(&hits, 1) == 1 {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	tr := NewTransport(TransportOptions{DSN: srv.URL, MaxBatchSize: 1})
	tr.Enqueue(ObservabilityEvent{EventID: "1"}) // full batch -> synchronous Flush

	// fetch retried the 500 in-call and succeeded, so the server saw >= 2 hits
	// and the queue drained without a re-queue.
	waitFor(t, func() bool { return atomic.LoadInt32(&hits) >= 2 })
	waitFor(t, func() bool { return tr.queueSize() == 0 })
}

// TestTransportSendErrorOnPersistent4xx confirms a persistent client error
// surfaces as httpStatusError (fetch aborts retries on 4xx) so Flush re-queues
// the batch rather than dropping it.
func TestTransportSendErrorOnPersistent4xx(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusBadRequest)
	}))
	defer srv.Close()

	tr := NewTransport(TransportOptions{DSN: srv.URL, MaxBatchSize: 10})
	tr.Enqueue(ObservabilityEvent{EventID: "1"})
	err := tr.Flush(context.Background())
	var statusErr *httpStatusError
	if !errors.As(err, &statusErr) {
		t.Fatalf("expected *httpStatusError, got %v", err)
	}
	if statusErr.status != http.StatusBadRequest {
		t.Errorf("status = %d, want 400", statusErr.status)
	}
	// Batch was re-queued for a later attempt, not dropped.
	if tr.queueSize() != 1 {
		t.Errorf("queue size = %d, want 1 (re-queued)", tr.queueSize())
	}
}

func waitFor(t *testing.T, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("condition not met within timeout")
}
