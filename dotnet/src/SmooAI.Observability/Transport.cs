using System.Net.Http;
using System.Text;

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
}

/// <summary>
/// Batched HTTP transport. Holds a small bounded queue, flushes on a timer or
/// when <see cref="TransportOptions.MaxBatchSize"/> events are buffered, and
/// retries a failed batch by pushing it back to the front of the queue. Errors
/// are swallowed — observability must never throw into the host application.
///
/// Port of the TS <c>Transport</c> (without the browser <c>sendBeacon</c> path,
/// which has no .NET analogue).
/// </summary>
public sealed class Transport : IDisposable, IAsyncDisposable
{
    private readonly TransportOptions _options;
    private readonly HttpClient _httpClient;
    private readonly bool _ownsHttpClient;
    private readonly Queue<ObservabilityEvent> _queue = new();
    private readonly object _gate = new();
    private readonly Timer _timer;
    private bool _inFlight;
    private bool _disposed;

    /// <summary>
    /// Create a transport. If <paramref name="httpClient"/> is null a private
    /// <see cref="HttpClient"/> is created and disposed with this transport.
    /// </summary>
    public Transport(TransportOptions options, HttpClient? httpClient = null)
    {
        _options = options ?? throw new ArgumentNullException(nameof(options));
        if (string.IsNullOrEmpty(_options.Dsn))
        {
            throw new ArgumentException("Transport requires a DSN.", nameof(options));
        }
        _ownsHttpClient = httpClient is null;
        _httpClient = httpClient ?? new HttpClient();
        _httpClient.Timeout = _options.RequestTimeout;
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
            using var content = new StringContent(json, Encoding.UTF8, "application/json");
            using var response = await _httpClient.PostAsync(_options.Dsn, content).ConfigureAwait(false);
            // Non-2xx is a delivery failure: requeue for the next attempt.
            if (!response.IsSuccessStatusCode)
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
            _httpClient.Dispose();
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
            _httpClient.Dispose();
        }
    }
}
