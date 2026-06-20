using System.Net;
using System.Net.Http;
using SmooAI.Observability.Auth;

namespace SmooAI.Observability.Tests;

public class TokenProviderTests
{
    private static StubHttpMessageHandler TokenStub(string token, int expiresIn = 3600) =>
        StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK,
            $"{{\"access_token\":\"{token}\",\"expires_in\":{expiresIn}}}");

    private static TokenProvider NewProvider(StubHttpMessageHandler stub) =>
        new(new TokenProviderOptions
        {
            AuthUrl = "https://auth.test",
            ClientId = "cid",
            ClientSecret = "sk_secret",
        }, new HttpClient(stub));

    [Fact]
    public async Task GetAccessToken_MintsAndCaches()
    {
        var stub = TokenStub("tok-1");
        var provider = NewProvider(stub);

        var first = await provider.GetAccessTokenAsync();
        var second = await provider.GetAccessTokenAsync();

        Assert.Equal("tok-1", first);
        Assert.Equal("tok-1", second);
        Assert.Equal(1, stub.CallCount); // cached, no second mint

        // Verify the client_credentials grant shape.
        Assert.Contains("grant_type=client_credentials", stub.CapturedBodies[0]);
        Assert.Contains("provider=client_credentials", stub.CapturedBodies[0]);
        Assert.Contains("client_id=cid", stub.CapturedBodies[0]);
    }

    [Fact]
    public async Task GetAccessToken_RefreshesWithinExpiryWindow()
    {
        var stub = TokenStub("tok-1", expiresIn: 100);
        var provider = NewProvider(stub);
        var now = 1000L;
        provider.SetNowForTests(() => now);

        var first = await provider.GetAccessTokenAsync(); // expiresAt = 1100
        Assert.Equal("tok-1", first);

        // Advance to within the 60s refresh window (1100 - 60 = 1041).
        now = 1050;
        var refreshed = await provider.GetAccessTokenAsync();

        Assert.Equal("tok-1", refreshed);
        Assert.Equal(2, stub.CallCount); // re-minted
    }

    [Fact]
    public async Task Invalidate_ForcesRemint()
    {
        var stub = TokenStub("tok-1");
        var provider = NewProvider(stub);

        await provider.GetAccessTokenAsync();
        provider.Invalidate();
        await provider.GetAccessTokenAsync();

        Assert.Equal(2, stub.CallCount);
    }

    [Fact]
    public async Task ConcurrentCallers_ShareSingleMint()
    {
        var stub = new StubHttpMessageHandler((_, _) =>
        {
            Thread.Sleep(50); // hold the mint so concurrent callers pile up
            return new HttpResponseMessage(HttpStatusCode.OK)
            {
                Content = new StringContent("{\"access_token\":\"tok\",\"expires_in\":3600}"),
            };
        });
        var provider = NewProvider(stub);

        var tasks = Enumerable.Range(0, 10).Select(_ => provider.GetAccessTokenAsync());
        var results = await Task.WhenAll(tasks);

        Assert.All(results, r => Assert.Equal("tok", r));
        Assert.Equal(1, stub.CallCount);
    }

    [Fact]
    public async Task FailedMint_Throws()
    {
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.Unauthorized, "nope");
        var provider = NewProvider(stub);
        await Assert.ThrowsAsync<InvalidOperationException>(() => provider.GetAccessTokenAsync());
    }

    [Fact]
    public void Constructor_ValidatesRequiredOptions()
    {
        Assert.Throws<ArgumentException>(() => new TokenProvider(new TokenProviderOptions { ClientId = "c", ClientSecret = "s" }));
        Assert.Throws<ArgumentException>(() => new TokenProvider(new TokenProviderOptions { AuthUrl = "u", ClientSecret = "s" }));
        Assert.Throws<ArgumentException>(() => new TokenProvider(new TokenProviderOptions { AuthUrl = "u", ClientId = "c" }));
    }
}
