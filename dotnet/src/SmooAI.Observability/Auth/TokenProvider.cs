using System.Net.Http;
using System.Text.Json;

namespace SmooAI.Observability.Auth;

/// <summary>
/// Options for <see cref="TokenProvider"/>.
/// </summary>
public sealed class TokenProviderOptions
{
    /// <summary>OAuth issuer base URL (no trailing slash required). E.g. <c>https://auth.smoo.ai</c>.</summary>
    public string AuthUrl { get; set; } = string.Empty;

    /// <summary>OAuth client id.</summary>
    public string ClientId { get; set; } = string.Empty;

    /// <summary>OAuth client secret (the <c>sk_*</c> minted by Smoo's M2M flow).</summary>
    public string ClientSecret { get; set; } = string.Empty;

    /// <summary>Seconds before expiry to proactively refresh. Defaults to 60s.</summary>
    public int RefreshWindowSec { get; set; } = 60;
}

/// <summary>
/// OAuth2 <c>client_credentials</c> token provider — direct port of the TS SDK's
/// <c>TokenProvider</c> so the .NET SDK authenticates against api.smoo.ai exactly
/// like every other SmooAI client. The token is cached in memory until 60s before
/// expiry, then refreshed; concurrent callers during a refresh share one in-flight
/// request. Consulted at request time by <see cref="Otel.AuthHeaderHandler"/> — no
/// header snapshot, no staleness.
/// </summary>
public sealed class TokenProvider
{
    private readonly string _authUrl;
    private readonly string _clientId;
    private readonly string _clientSecret;
    private readonly int _refreshWindowSec;
    private readonly HttpClient _httpClient;
    private readonly bool _ownsHttpClient;
    private readonly SemaphoreSlim _refreshGate = new(1, 1);

    private string? _accessToken;
    private long _expiresAtUnixSec;
    private Func<long> _nowUnixSec = () => DateTimeOffset.UtcNow.ToUnixTimeSeconds();

    /// <summary>
    /// Create a token provider. If <paramref name="httpClient"/> is null a private
    /// <see cref="HttpClient"/> is created and disposed with this provider.
    /// </summary>
    public TokenProvider(TokenProviderOptions options, HttpClient? httpClient = null)
    {
        ArgumentNullException.ThrowIfNull(options);
        if (string.IsNullOrEmpty(options.AuthUrl))
        {
            throw new ArgumentException("TokenProvider requires authUrl.", nameof(options));
        }
        if (string.IsNullOrEmpty(options.ClientId))
        {
            throw new ArgumentException("TokenProvider requires clientId.", nameof(options));
        }
        if (string.IsNullOrEmpty(options.ClientSecret))
        {
            throw new ArgumentException("TokenProvider requires clientSecret.", nameof(options));
        }
        _authUrl = options.AuthUrl.TrimEnd('/');
        _clientId = options.ClientId;
        _clientSecret = options.ClientSecret;
        _refreshWindowSec = options.RefreshWindowSec;
        _ownsHttpClient = httpClient is null;
        _httpClient = httpClient ?? new HttpClient();
    }

    /// <summary>
    /// Returns a valid access token, refreshing if the cached value is missing,
    /// expired, or within the refresh window. Concurrent callers during a refresh
    /// share a single token exchange.
    /// </summary>
    public async Task<string> GetAccessTokenAsync(CancellationToken cancellationToken = default)
    {
        if (!ShouldRefresh())
        {
            return _accessToken!;
        }

        await _refreshGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            // Re-check under the lock — another caller may have refreshed.
            if (!ShouldRefresh())
            {
                return _accessToken!;
            }
            return await RefreshAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _refreshGate.Release();
        }
    }

    /// <summary>
    /// Drop the cached token. Call when an export observes a 401 so the next
    /// attempt re-mints.
    /// </summary>
    public void Invalidate()
    {
        _accessToken = null;
        _expiresAtUnixSec = 0;
    }

    private bool ShouldRefresh()
    {
        if (_accessToken is null)
        {
            return true;
        }
        return _nowUnixSec() >= _expiresAtUnixSec - _refreshWindowSec;
    }

    private async Task<string> RefreshAsync(CancellationToken cancellationToken)
    {
        using var content = new FormUrlEncodedContent(new[]
        {
            new KeyValuePair<string, string>("grant_type", "client_credentials"),
            new KeyValuePair<string, string>("provider", "client_credentials"),
            new KeyValuePair<string, string>("client_id", _clientId),
            new KeyValuePair<string, string>("client_secret", _clientSecret),
        });

        using var response = await _httpClient.PostAsync($"{_authUrl}/token", content, cancellationToken).ConfigureAwait(false);
        if (!response.IsSuccessStatusCode)
        {
            var body = await SafeReadBodyAsync(response).ConfigureAwait(false);
            throw new InvalidOperationException($"OAuth token exchange failed: HTTP {(int)response.StatusCode} {body}");
        }

        var json = await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        if (!root.TryGetProperty("access_token", out var tokenEl) || tokenEl.ValueKind != JsonValueKind.String)
        {
            throw new InvalidOperationException("OAuth token endpoint returned no access_token.");
        }

        var token = tokenEl.GetString()!;
        var expiresIn = root.TryGetProperty("expires_in", out var expEl) && expEl.ValueKind == JsonValueKind.Number
            ? expEl.GetInt64()
            : 3600;

        _accessToken = token;
        _expiresAtUnixSec = _nowUnixSec() + expiresIn;
        return token;
    }

    private static async Task<string> SafeReadBodyAsync(HttpResponseMessage response)
    {
        try
        {
            return await response.Content.ReadAsStringAsync().ConfigureAwait(false);
        }
        catch
        {
            return "<unreadable>";
        }
    }

    /// <summary>Test seam — override the time source.</summary>
    internal void SetNowForTests(Func<long> nowUnixSec) => _nowUnixSec = nowUnixSec;

    /// <summary>Dispose the owned HttpClient, if any.</summary>
    internal void DisposeOwned()
    {
        if (_ownsHttpClient)
        {
            _httpClient.Dispose();
        }
        _refreshGate.Dispose();
    }
}
