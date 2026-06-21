using System.Net;
using System.Net.Http;
using SmooAI.Observability;

namespace SmooAI.Observability.Tests;

public class TransportTests
{
    private static ObservabilityEvent NewEvent(string id) => new()
    {
        EventId = id,
        Timestamp = 1,
        Level = Level.Error,
        Sdk = new SdkInfo { Name = "test", Version = "0", Runtime = Runtime.Node },
    };

    [Fact]
    public async Task FlushAsync_PostsBatchAsErrorPayload()
    {
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        using var http = new HttpClient(stub);
        await using var transport = new Transport(new TransportOptions { Dsn = "https://ingest.test/hook" }, http);

        transport.Enqueue(NewEvent("a"));
        transport.Enqueue(NewEvent("b"));
        await transport.FlushAsync();

        Assert.Equal(1, stub.CallCount);
        Assert.Contains("\"type\":\"error\"", stub.CapturedBodies[0]);
        Assert.Contains("\"eventId\":\"a\"", stub.CapturedBodies[0]);
        Assert.Contains("\"eventId\":\"b\"", stub.CapturedBodies[0]);
        Assert.Equal(0, transport.QueueSize);
    }

    [Fact]
    public async Task Enqueue_FlushesImmediatelyAtBatchSize()
    {
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        using var http = new HttpClient(stub);
        await using var transport = new Transport(
            new TransportOptions { Dsn = "https://ingest.test/hook", MaxBatchSize = 2 },
            http);

        transport.Enqueue(NewEvent("a"));
        transport.Enqueue(NewEvent("b")); // hits batch size -> immediate flush

        // Give the fire-and-forget flush a moment.
        await WaitFor(() => stub.CallCount >= 1);
        Assert.Equal(1, stub.CallCount);
    }

    [Fact]
    public async Task FlushAsync_RequeuesOnServerError()
    {
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.InternalServerError);
        using var http = new HttpClient(stub);
        await using var transport = new Transport(new TransportOptions { Dsn = "https://ingest.test/hook" }, http);

        transport.Enqueue(NewEvent("a"));
        await transport.FlushAsync();

        // Failed batch is restored to the queue for a later attempt.
        Assert.Equal(1, transport.QueueSize);
    }

    [Fact]
    public async Task FlushAsync_RequeuesOnNetworkException()
    {
        var stub = new StubHttpMessageHandler((_, _) => throw new HttpRequestException("network down"));
        using var http = new HttpClient(stub);
        await using var transport = new Transport(new TransportOptions { Dsn = "https://ingest.test/hook" }, http);

        transport.Enqueue(NewEvent("a"));
        await transport.FlushAsync(); // must not throw

        Assert.Equal(1, transport.QueueSize);
    }

    [Fact]
    public void Enqueue_DropsOldestWhenQueueFull()
    {
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        using var http = new HttpClient(stub);
        // Large batch size so enqueue never auto-flushes; tiny queue cap.
        using var transport = new Transport(
            new TransportOptions { Dsn = "https://ingest.test/hook", MaxQueueSize = 2, MaxBatchSize = 100 },
            http);

        transport.Enqueue(NewEvent("a"));
        transport.Enqueue(NewEvent("b"));
        transport.Enqueue(NewEvent("c")); // evicts "a"

        Assert.Equal(2, transport.QueueSize);
    }

    [Fact]
    public void Constructor_RejectsEmptyDsn()
    {
        Assert.Throws<ArgumentException>(() => new Transport(new TransportOptions { Dsn = "" }));
    }

    // --- Default transport (SmooFetch) path ----------------------------------
    //
    // The default Transport (no caller-supplied HttpClient) delivers through
    // SmooFetch, which owns its own real HttpClient and cannot accept a stub
    // handler. These tests exercise it end-to-end against a loopback HTTP server
    // so the SmooFetch retry/timeout/circuit-breaking pipeline is the code under
    // test, not a mock.

    [Fact]
    public async Task DefaultTransport_DeliversBatchViaSmooFetch()
    {
        using var server = new LoopbackServer(_ => HttpStatusCode.OK);
        await using var transport = new Transport(new TransportOptions { Dsn = server.Url });

        transport.Enqueue(NewEvent("a"));
        transport.Enqueue(NewEvent("b"));
        await transport.FlushAsync();

        await WaitFor(() => server.RequestCount >= 1);
        Assert.Equal(1, server.RequestCount);
        Assert.Contains("\"type\":\"error\"", server.LastBody);
        Assert.Contains("\"eventId\":\"a\"", server.LastBody);
        Assert.Equal(0, transport.QueueSize);
    }

    [Fact]
    public async Task DefaultTransport_RequeuesAfterRetriesExhausted()
    {
        // Always 500: SmooFetch retries within the flush, then PostAsync reports
        // a failed delivery and the batch is requeued.
        using var server = new LoopbackServer(_ => HttpStatusCode.InternalServerError);
        await using var transport = new Transport(
            new TransportOptions { Dsn = server.Url, MaxRetries = 1 });

        transport.Enqueue(NewEvent("a"));
        await transport.FlushAsync();

        Assert.Equal(1, transport.QueueSize);
        // Initial attempt + 1 retry = 2 requests reached the server.
        Assert.True(server.RequestCount >= 2, $"expected >=2 requests, saw {server.RequestCount}");
    }

    [Fact]
    public async Task DefaultTransport_RequeuesWhenEndpointUnreachable()
    {
        // Reserve and immediately release a port so connections are refused.
        var url = LoopbackServer.ReserveUnusedUrl();
        await using var transport = new Transport(
            new TransportOptions { Dsn = url, MaxRetries = 0, RequestTimeout = TimeSpan.FromSeconds(2) });

        transport.Enqueue(NewEvent("a"));
        await transport.FlushAsync(); // must not throw

        Assert.Equal(1, transport.QueueSize);
    }

    private static async Task WaitFor(Func<bool> condition, int timeoutMs = 2000)
    {
        var deadline = DateTime.UtcNow.AddMilliseconds(timeoutMs);
        while (DateTime.UtcNow < deadline)
        {
            if (condition())
            {
                return;
            }
            await Task.Delay(10);
        }
    }
}
