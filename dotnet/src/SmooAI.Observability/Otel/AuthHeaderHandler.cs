using System.Net.Http;
using System.Net.Http.Headers;
using SmooAI.Observability.Auth;

namespace SmooAI.Observability.Otel;

/// <summary>
/// Delegating handler that injects a fresh <c>Authorization: Bearer</c> on every
/// OTLP export request from a <see cref="TokenProvider"/>, and on a 401 drops the
/// cached token and retries once. This is the .NET answer to the same problem the
/// TS <c>AuthInjectingTraceExporter</c> solves: the token must be resolved per
/// request, not snapshotted at exporter construction, or every export 401s after
/// the first token expires (~1h).
///
/// Wired into the OTLP exporter via
/// <c>OtlpExporterOptions.HttpClientFactory</c>.
/// </summary>
public sealed class AuthHeaderHandler : DelegatingHandler
{
    private readonly TokenProvider _tokenProvider;

    /// <summary>Create the handler over the given token provider.</summary>
    public AuthHeaderHandler(TokenProvider tokenProvider)
        : base(new HttpClientHandler())
    {
        _tokenProvider = tokenProvider ?? throw new ArgumentNullException(nameof(tokenProvider));
    }

    /// <inheritdoc />
    protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        await ApplyTokenAsync(request, cancellationToken).ConfigureAwait(false);
        var response = await base.SendAsync(request, cancellationToken).ConfigureAwait(false);

        if (response.StatusCode == System.Net.HttpStatusCode.Unauthorized)
        {
            // Token rotated / revoked — re-mint once and retry.
            response.Dispose();
            _tokenProvider.Invalidate();
            await ApplyTokenAsync(request, cancellationToken).ConfigureAwait(false);
            response = await base.SendAsync(request, cancellationToken).ConfigureAwait(false);
        }

        return response;
    }

    private async Task ApplyTokenAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        try
        {
            var token = await _tokenProvider.GetAccessTokenAsync(cancellationToken).ConfigureAwait(false);
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
        }
        catch
        {
            // Mint failure — let the request go unauthenticated; the exporter will
            // log/retry. Observability must not throw into the host.
        }
    }
}
