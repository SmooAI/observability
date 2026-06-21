using System.Net;

namespace SmooAI.Observability.Tests;

/// <summary>
/// Minimal in-process HTTP server for exercising the default SmooFetch transport
/// path (which owns a real <see cref="HttpClient"/> and cannot take a stub
/// handler). Listens on a free loopback port, records each request body, and
/// replies with a caller-chosen status code.
/// </summary>
public sealed class LoopbackServer : IDisposable
{
    private readonly HttpListener _listener;
    private readonly Func<string, HttpStatusCode> _responder;
    private readonly CancellationTokenSource _cts = new();
    private int _requestCount;

    public LoopbackServer(Func<string, HttpStatusCode> responder)
    {
        _responder = responder;
        var port = GetFreePort();
        Url = $"http://127.0.0.1:{port}/ingest";
        _listener = new HttpListener();
        _listener.Prefixes.Add($"http://127.0.0.1:{port}/");
        _listener.Start();
        _ = Task.Run(AcceptLoopAsync);
    }

    /// <summary>Full URL (including path) to POST to.</summary>
    public string Url { get; }

    /// <summary>Number of requests received so far.</summary>
    public int RequestCount => Volatile.Read(ref _requestCount);

    /// <summary>Body of the most recently received request.</summary>
    public string LastBody { get; private set; } = string.Empty;

    private async Task AcceptLoopAsync()
    {
        while (!_cts.IsCancellationRequested)
        {
            HttpListenerContext context;
            try
            {
                context = await _listener.GetContextAsync().ConfigureAwait(false);
            }
            catch
            {
                return; // listener stopped
            }

            try
            {
                using var reader = new StreamReader(context.Request.InputStream, context.Request.ContentEncoding);
                var body = await reader.ReadToEndAsync().ConfigureAwait(false);
                LastBody = body;
                Interlocked.Increment(ref _requestCount);

                var status = _responder(body);
                context.Response.StatusCode = (int)status;
                context.Response.Close();
            }
            catch
            {
                try
                {
                    context.Response.Abort();
                }
                catch
                {
                    // ignore
                }
            }
        }
    }

    /// <summary>Reserve a free port, release it, and return a URL that will refuse connections.</summary>
    public static string ReserveUnusedUrl() => $"http://127.0.0.1:{GetFreePort()}/ingest";

    private static int GetFreePort()
    {
        var listener = new System.Net.Sockets.TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var port = ((IPEndPoint)listener.LocalEndpoint).Port;
        listener.Stop();
        return port;
    }

    public void Dispose()
    {
        _cts.Cancel();
        try
        {
            _listener.Stop();
            _listener.Close();
        }
        catch
        {
            // ignore
        }
        _cts.Dispose();
    }
}
