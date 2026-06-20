using System.Net;
using System.Net.Http;

namespace SmooAI.Observability.Tests;

/// <summary>
/// Test double for <see cref="HttpMessageHandler"/> — records requests (with
/// their body captured before disposal) and returns scripted responses.
/// </summary>
public sealed class StubHttpMessageHandler : HttpMessageHandler
{
    private readonly Func<HttpRequestMessage, string, HttpResponseMessage> _responder;

    public List<string> CapturedBodies { get; } = new();
    public List<HttpRequestMessage> CapturedRequests { get; } = new();
    public int CallCount => CapturedRequests.Count;

    public StubHttpMessageHandler(Func<HttpRequestMessage, string, HttpResponseMessage> responder)
    {
        _responder = responder;
    }

    public static StubHttpMessageHandler AlwaysStatus(HttpStatusCode status, string body = "{}") =>
        new((_, _) => new HttpResponseMessage(status) { Content = new StringContent(body) });

    protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var body = request.Content is null
            ? string.Empty
            : await request.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
        lock (CapturedRequests)
        {
            CapturedRequests.Add(request);
            CapturedBodies.Add(body);
        }
        return _responder(request, body);
    }
}
