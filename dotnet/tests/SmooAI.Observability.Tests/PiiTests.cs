using SmooAI.Observability;

namespace SmooAI.Observability.Tests;

public class PiiTests
{
    [Fact]
    public void ScrubString_RedactsBearerTokens()
    {
        var result = Pii.ScrubString("auth header: Bearer abc.def-123_XYZ");
        Assert.Contains("Bearer [redacted]", result);
        Assert.DoesNotContain("abc.def-123_XYZ", result);
    }

    [Fact]
    public void ScrubString_RedactsPassword()
    {
        var result = Pii.ScrubString("login password=hunter2 done");
        Assert.Contains("password=[redacted]", result);
        Assert.DoesNotContain("hunter2", result);
    }

    [Theory]
    [InlineData("token=secretvalue")]
    [InlineData("api_key: abc123")]
    [InlineData("apikey=xyz")]
    [InlineData("secret = topsecret")]
    public void ScrubString_RedactsSecretValues(string input)
    {
        var result = Pii.ScrubString(input);
        Assert.Contains("[redacted]", result);
    }

    [Fact]
    public void ScrubString_RedactsSkKeys()
    {
        var result = Pii.ScrubString("key sk-ABCDEFGHIJKLMNOPQRSTUVWX rest");
        Assert.Contains("sk-[redacted]", result);
        Assert.DoesNotContain("ABCDEFGHIJKLMNOPQRSTUVWX", result);
    }

    [Fact]
    public void ScrubString_LeavesCleanStringUntouched()
    {
        const string clean = "nothing sensitive here";
        Assert.Equal(clean, Pii.ScrubString(clean));
    }

    [Fact]
    public void ScrubString_HandlesNullAndEmpty()
    {
        Assert.Equal(string.Empty, Pii.ScrubString(null));
        Assert.Equal(string.Empty, Pii.ScrubString(string.Empty));
    }

    [Fact]
    public void ScrubHeaders_FullyRedactsSensitiveHeaders()
    {
        var headers = new Dictionary<string, string>
        {
            ["Authorization"] = "Bearer xyz",
            ["Cookie"] = "session=abc",
            ["X-Api-Key"] = "key",
            ["Content-Type"] = "application/json",
        };

        var scrubbed = Pii.ScrubHeaders(headers);

        Assert.NotNull(scrubbed);
        Assert.Equal("[redacted]", scrubbed!["Authorization"]);
        Assert.Equal("[redacted]", scrubbed["Cookie"]);
        Assert.Equal("[redacted]", scrubbed["X-Api-Key"]);
        Assert.Equal("application/json", scrubbed["Content-Type"]);
    }

    [Fact]
    public void ScrubHeaders_IsCaseInsensitiveOnHeaderName()
    {
        var headers = new Dictionary<string, string> { ["AUTHORIZATION"] = "Bearer xyz" };
        var scrubbed = Pii.ScrubHeaders(headers);
        Assert.Equal("[redacted]", scrubbed!["AUTHORIZATION"]);
    }

    [Fact]
    public void ScrubHeaders_ReturnsNullForNull()
    {
        Assert.Null(Pii.ScrubHeaders(null));
    }
}
