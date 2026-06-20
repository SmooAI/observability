package observability

import (
	"context"
	"encoding/json"
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
