using System.Net.Http;
using System.Text;
using SmooAI.Fetch;

namespace SmooAI.Observability;

/// <summary>
/// Tunables for <see cref="Transport"/>. Defaults match the TS SDK.
/// </summary>
public sealed class TransportOptions
{
    /// <summary>Ingest endpoint: <c>POST /webhooks/observability/{org_id}/{token}</c>.</summary>
    public string Dsn { get; set; } = string.Empty;

    /// <summary>Flush interval in ms. Default 1000.</summary>
    public int FlushIntervalMs { get; set; } = 1000;

    /// <summary>Max events per flush batch. Default 30.</summary>
    public int MaxBatchSize { get; set; } = 30;

    /// <summary>Max events kept in memory waiting to flush. Default 250.</summary>
    public int MaxQueueSize { get; set; } = 250;

    /// <summary>Per-request timeout. Default 10s.</summary>
    public TimeSpan RequestTimeout { get; set; } = TimeSpan.FromSeconds(10);

    /// <summary>
    /// Retries applied <em>within a single flush attempt</em> by the resilient
    /// <see cref="SmooFetch"/> transport (transient exceptions + 429/5xx). This is
    /// distinct from the transport's own batch-requeue, which only kicks in once a
    /// flush has exhausted these retries and still failed. Default 2.
    /// </summary>
    public int MaxRetries { get; set; } = 2;
}

/// <summary>
/// Batched HTTP transport. Holds a small bounded queue, flushes on a timer or
/// when <see cref="TransportOptions.MaxBatchSize"/> events are buffered, and
/// retries a failed batch by pushing it back to the front of the queue. Errors
/// are swallowed — observability must never throw into the host application.
///
/// Outbound delivery goes through <see cref="SmooFetch"/> (the SmooAI resilient
/// fetch — Polly-backed retry, per-request timeout, circuit breaking) by default.
/// A caller-supplied <see cref="HttpClient"/> is honored as an escape hatch and
/// used directly (no SmooFetch resilience layer), for tests or hosts that need a
/// fully custom pipeline.
///
/// Port of the TS <c>Transport</c> (without the browser <c>sendBeacon</c> path,
/// which has no .NET analogue).
/// </summary>
public sealed class Transport : IDisposable, IAsyncDisposable
{
    private readonly TransportOptions _options;
    // Exactly one of these is set: _fetch by default, _httpClient when the caller
    // supplies a custom client via the escape hatch.
    private readonly SmooFetch? _fetch;
    private readonly HttpClient? _httpClient;
    private readonly bool _ownsHttpClient;
    private readonly Queue<ObservabilityEvent> _queue = new();
    private readonly object _gate = new();
    private readonly Timer _timer;
    private bool _inFlight;
    private bool _disposed;

    /// <summary>
    /// Create a transport. By default delivery uses an internally-managed,
    /// resilient <see cref="SmooFetch"/>. If <paramref name="httpClient"/> is
    /// supplied it is used directly as an escape hatch — the caller owns its
    /// disposal and is responsible for any resilience behavior.
    /// </summary>
    public Transport(TransportOptions options, HttpClient? httpClient = null)
    {
        _options = options ?? throw new ArgumentNullException(nameof(options));
        if (string.IsNullOrEmpty(_options.Dsn))
        {
            throw new ArgumentException("Transport requires a DSN.", nameof(options));
        }

        if (httpClient is null)
        {
            // Default path: resilient SmooFetch with the transport's timeout +
            // retry config and a circuit breaker so a hard-down ingest endpoint
            // stops hammering after repeated failures.
            _fetch = SmooFetch.Create(o =>
            {
                o.Timeout = _options.RequestTimeout;
                o.RetryPolicy = _options.MaxRetries > 0
                    ? RetryPolicy.ExponentialBackoff(_options.MaxRetries)
                    : RetryPolicy.None;
                // Stop hammering a hard-down ingest endpoint: open after 5
                // consecutive failures for 30s. A tripped breaker throws, which
                // PostAsync catches and reports as a failed delivery, so the
                // batch is requeued rather than lost.
                o.CircuitBreaker = new CircuitBreakerOptions(
                    FailureThreshold: 5,
                    OpenDuration: TimeSpan.FromSeconds(30));
            });
            _ownsHttpClient = false;
        }
        else
        {
            // Escape hatch: use the caller's client verbatim.
            _httpClient = httpClient;
            _httpClient.Timeout = _options.RequestTimeout;
            _ownsHttpClient = false;
        }

        // Disarmed timer; armed on demand when the first event is enqueued.
        _timer = new Timer(_ => _ = FlushAsync(), null, Timeout.Infinite, Timeout.Infinite);
    }

    /// <summary>
    /// Queue an event for delivery. Drops the oldest event if the queue is full
    /// (recent events are more useful). Triggers an immediate flush once the
    /// batch threshold is reached, otherwise arms the flush timer.
    /// </summary>
    public void Enqueue(ObservabilityEvent ev)
    {
        if (ev is null || _disposed)
        {
            return;
        }

        bool flushNow = false;
        lock (_gate)
        {
            if (_queue.Count >= _options.MaxQueueSize)
            {
                _queue.Dequeue();
            }
            _queue.Enqueue(ev);

            if (_queue.Count >= _options.MaxBatchSize)
            {
                flushNow = true;
            }
            else
            {
                ArmTimer();
            }
        }

        if (flushNow)
        {
            _ = FlushAsync();
        }
    }

    /// <summary>
    /// Flush up to <see cref="TransportOptions.MaxBatchSize"/> queued events.
    /// On failure the batch is restored to the front of the queue and the timer
    /// re-armed. Never throws.
    /// </summary>
    public async Task FlushAsync()
    {
        List<ObservabilityEvent> batch;
        lock (_gate)
        {
            if (_inFlight || _queue.Count == 0)
            {
                DisarmTimer();
                return;
            }
            _inFlight = true;
            batch = DequeueBatch();
            DisarmTimer();
        }

        try
        {
            var payload = new IngestPayload { Type = "error", Events = batch };
            var json = ObservabilityJson.Serialize(payload);
            var delivered = await PostAsync(json).ConfigureAwait(false);
            // Non-2xx (or transport failure) is a delivery failure: requeue for
            // the next attempt.
            if (!delivered)
            {
                RestoreBatch(batch);
            }
        }
        catch
        {
            // Network / serialization failure — best-effort requeue.
            RestoreBatch(batch);
        }
        finally
        {
            lock (_gate)
            {
                _inFlight = false;
                if (_queue.Count > 0)
                {
                    ArmTimer();
                }
            }
        }
    }

    /// <summary>
    /// POST the serialized payload via SmooFetch (default) or the escape-hatch
    /// <see cref="HttpClient"/>. Returns <c>true</c> on a 2xx, <c>false</c>
    /// otherwise. Never throws (transport exceptions are caught and reported as a
    /// failed delivery so the batch is requeued).
    /// </summary>
    private async Task<bool> PostAsync(string json)
    {
        if (_fetch is not null)
        {
            // SmooFetch serializes typed bodies itself, but we need the exact
            // ObservabilityJson wire bytes (camelCase + omit-nulls), so build the
            // request with pre-serialized content and use the low-level SendAsync
            // (which applies retry/timeout/circuit-breaking but does NOT throw on
            // non-2xx — we inspect the status ourselves).
            using var request = new HttpRequestMessage(HttpMethod.Post, _options.Dsn)
            {
                Content = new StringContent(json, Encoding.UTF8, "application/json"),
            };
            try
            {
                using var response = await _fetch.SendAsync(request).ConfigureAwait(false);
                return response.IsSuccessStatusCode;
            }
            catch
            {
                // Retries / circuit breaker exhausted — treat as a failed delivery.
                return false;
            }
        }

        // Escape hatch: raw HttpClient, no SmooFetch resilience.
        using var content = new StringContent(json, Encoding.UTF8, "application/json");
        using var raw = await _httpClient!.PostAsync(_options.Dsn, content).ConfigureAwait(false);
        return raw.IsSuccessStatusCode;
    }

    /// <summary>Current queue depth — exposed for tests.</summary>
    public int QueueSize
    {
        get
        {
            lock (_gate)
            {
                return _queue.Count;
            }
        }
    }

    private List<ObservabilityEvent> DequeueBatch()
    {
        var take = Math.Min(_options.MaxBatchSize, _queue.Count);
        var batch = new List<ObservabilityEvent>(take);
        for (var i = 0; i < take; i++)
        {
            batch.Add(_queue.Dequeue());
        }
        return batch;
    }

    private void RestoreBatch(List<ObservabilityEvent> batch)
    {
        lock (_gate)
        {
            // Rebuild the queue with the failed batch at the front, preserving order.
            var pending = _queue.ToArray();
            _queue.Clear();
            foreach (var ev in batch)
            {
                _queue.Enqueue(ev);
            }
            foreach (var ev in pending)
            {
                _queue.Enqueue(ev);
            }
            // Trim if restoring pushed us over the cap.
            while (_queue.Count > _options.MaxQueueSize)
            {
                _queue.Dequeue();
            }
        }
    }

    // Callers must hold _gate.
    private void ArmTimer()
    {
        if (!_disposed)
        {
            _timer.Change(_options.FlushIntervalMs, Timeout.Infinite);
        }
    }

    // Callers must hold _gate.
    private void DisarmTimer() => _timer.Change(Timeout.Infinite, Timeout.Infinite);

    /// <inheritdoc />
    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        // Best-effort final flush.
        try
        {
            FlushAsync().GetAwaiter().GetResult();
        }
        catch
        {
            // swallow
        }
        _timer.Dispose();
        if (_ownsHttpClient)
        {
            _httpClient?.Dispose();
        }
    }

    /// <inheritdoc />
    public async ValueTask DisposeAsync()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        try
        {
            await FlushAsync().ConfigureAwait(false);
        }
        catch
        {
            // swallow
        }
        await _timer.DisposeAsync().ConfigureAwait(false);
        if (_ownsHttpClient)
        {
            _httpClient?.Dispose();
        }
    }
}
