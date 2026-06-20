//! Batched HTTP transport for the Smoo error-ingest webhook.
//!
//! Holds a small in-memory queue, flushes on a timer or when `max_batch_size`
//! events are buffered, and POSTs `{ type: "error", events: [...] }` to the DSN
//! (`/webhooks/observability/{org_id}/{token}`) — byte-identical to what the TS
//! transport sends.
//!
//! Errors are swallowed — observability must never throw into host code. On a
//! failed flush the batch is pushed back to the FRONT of the queue for the next
//! attempt (matching the TS `queue.unshift(...batch)` retry behavior).

use crate::types::{IngestPayload, IngestType, ObservabilityEvent};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const DEFAULT_FLUSH_MS: u64 = 1000;
const DEFAULT_BATCH_SIZE: usize = 30;
const DEFAULT_QUEUE_MAX: usize = 250;

/// Tuning knobs. Mirrors the TS `ClientOptions` transport-relevant fields.
#[derive(Clone)]
pub struct TransportOptions {
    /// Ingest endpoint: `POST /webhooks/observability/{org_id}/{token}`.
    pub dsn: String,
    /// Max events kept in memory waiting to be flushed. Default 250.
    pub max_queue_size: usize,
    /// Flush interval. Default 1s.
    pub flush_interval: Duration,
    /// Max events per flush batch. Default 30.
    pub max_batch_size: usize,
}

impl TransportOptions {
    pub fn new(dsn: impl Into<String>) -> Self {
        TransportOptions {
            dsn: dsn.into(),
            max_queue_size: DEFAULT_QUEUE_MAX,
            flush_interval: Duration::from_millis(DEFAULT_FLUSH_MS),
            max_batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

struct TransportInner {
    opts: TransportOptions,
    http: reqwest::Client,
    queue: Mutex<VecDeque<ObservabilityEvent>>,
}

/// Universal batched transport. Cheap to clone (`Arc`-shared). A background
/// flush loop drains the queue on the configured interval; `enqueue` also
/// triggers an immediate flush once the batch threshold is reached.
#[derive(Clone)]
pub struct Transport {
    inner: Arc<TransportInner>,
    _flush_loop: Arc<FlushLoopGuard>,
}

struct FlushLoopGuard(Mutex<Option<JoinHandle<()>>>);

impl Drop for FlushLoopGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.0.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

impl Transport {
    /// Build a transport and spawn its background flush loop. Requires a tokio
    /// runtime (the loop is a spawned task).
    pub fn new(opts: TransportOptions) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        let inner = Arc::new(TransportInner {
            opts: opts.clone(),
            http,
            queue: Mutex::new(VecDeque::new()),
        });
        let loop_inner = inner.clone();
        let interval = opts.flush_interval;
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                flush_inner(&loop_inner).await;
            }
        });
        Transport {
            inner,
            _flush_loop: Arc::new(FlushLoopGuard(Mutex::new(Some(handle)))),
        }
    }

    /// Queue an event. Drops the oldest when the queue is full (recent events
    /// are more useful), and triggers an immediate flush once the batch size is
    /// reached.
    pub async fn enqueue(&self, event: ObservabilityEvent) {
        let should_flush = {
            let mut q = self.inner.queue.lock().await;
            if q.len() >= self.inner.opts.max_queue_size {
                q.pop_front();
            }
            q.push_back(event);
            q.len() >= self.inner.opts.max_batch_size
        };
        if should_flush {
            flush_inner(&self.inner).await;
        }
    }

    /// Flush one batch now. Best-effort; errors are swallowed.
    pub async fn flush(&self) {
        flush_inner(&self.inner).await;
    }

    /// Drain the ENTIRE queue, batch by batch. Call on graceful shutdown.
    pub async fn flush_all(&self) {
        loop {
            let empty = { self.inner.queue.lock().await.is_empty() };
            if empty {
                break;
            }
            flush_inner(&self.inner).await;
        }
    }

    /// Current queue depth — for tests + diagnostics.
    pub async fn queue_size(&self) -> usize {
        self.inner.queue.lock().await.len()
    }
}

async fn flush_inner(inner: &Arc<TransportInner>) {
    let batch: Vec<ObservabilityEvent> = {
        let mut q = inner.queue.lock().await;
        if q.is_empty() {
            return;
        }
        let take = inner.opts.max_batch_size.min(q.len());
        q.drain(0..take).collect()
    };

    let payload = IngestPayload {
        r#type: IngestType::Error,
        events: batch.clone(),
    };

    let result = inner
        .http
        .post(&inner.opts.dsn)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .await;

    let ok = matches!(result, Ok(ref res) if res.status().is_success());
    if !ok {
        // Push the batch back to the FRONT for the next attempt (matches the TS
        // `queue.unshift(...batch)`). Bounded by max_queue_size so a permanently
        // failing endpoint can't grow memory without limit.
        let mut q = inner.queue.lock().await;
        for event in batch.into_iter().rev() {
            if q.len() >= inner.opts.max_queue_size {
                q.pop_back();
            }
            q.push_front(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Level, Runtime, SdkInfo};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn event(id: &str) -> ObservabilityEvent {
        ObservabilityEvent {
            event_id: id.into(),
            timestamp: 0,
            level: Level::Error,
            message: Some("boom".into()),
            exception: None,
            breadcrumbs: None,
            user: None,
            request: None,
            tags: None,
            contexts: None,
            release: None,
            environment: None,
            sdk: SdkInfo {
                name: "@smooai/observability".into(),
                version: "0.1.0".into(),
                runtime: Runtime::Node,
            },
        }
    }

    #[tokio::test]
    async fn flushes_batch_to_webhook() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let mut opts = TransportOptions::new(format!("{}/hook", server.uri()));
        opts.max_batch_size = 2;
        let t = Transport::new(opts);
        t.enqueue(event("1")).await;
        assert_eq!(t.queue_size().await, 1);
        // Second enqueue hits the batch threshold → immediate flush.
        t.enqueue(event("2")).await;
        assert_eq!(t.queue_size().await, 0);
    }

    #[tokio::test]
    async fn requeues_on_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let mut opts = TransportOptions::new(format!("{}/hook", server.uri()));
        opts.max_batch_size = 1;
        let t = Transport::new(opts);
        t.enqueue(event("1")).await; // flush attempted, 500 → requeued
        assert_eq!(t.queue_size().await, 1, "failed batch should be requeued");
    }

    #[tokio::test]
    async fn drops_oldest_when_full() {
        let mut opts = TransportOptions::new("http://127.0.0.1:1/hook");
        opts.max_queue_size = 2;
        opts.max_batch_size = 100; // never auto-flush
        let t = Transport::new(opts);
        t.enqueue(event("1")).await;
        t.enqueue(event("2")).await;
        t.enqueue(event("3")).await; // evicts "1"
        assert_eq!(t.queue_size().await, 2);
    }

    #[tokio::test]
    async fn flush_all_drains_queue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut opts = TransportOptions::new(format!("{}/hook", server.uri()));
        opts.max_batch_size = 2;
        let t = Transport::new(opts);
        for i in 0..5 {
            // Use a high batch threshold path: enqueue without tripping flush by
            // setting batch=2 means some auto-flush; flush_all cleans the rest.
            t.enqueue(event(&i.to_string())).await;
        }
        t.flush_all().await;
        assert_eq!(t.queue_size().await, 0);
    }
}
