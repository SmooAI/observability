using System.Net;
using System.Net.Http;
using System.Text.Json;
using SmooAI.Observability;

namespace SmooAI.Observability.Tests;

public class ClientTests
{
    private static ObservabilityClient NewClient(StubHttpMessageHandler stub, Action<ClientOptions>? configure = null)
    {
        var client = new ObservabilityClient();
        var options = new ClientOptions
        {
            Dsn = "https://ingest.test/hook",
            Environment = "test",
            Release = "abc123",
            FlushIntervalMs = 50,
            HttpClient = new HttpClient(stub),
        };
        configure?.Invoke(options);
        client.Init(options);
        return client;
    }

    [Fact]
    public async Task CaptureException_ReturnsEventIdAndDelivers()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);

        string? id;
        try
        {
            throw new InvalidOperationException("kaboom");
        }
        catch (Exception ex)
        {
            id = client.CaptureException(ex);
        }

        Assert.False(string.IsNullOrEmpty(id));
        await client.FlushAsync();
        Assert.Equal(1, stub.CallCount);

        var payload = stub.CapturedBodies[0];
        Assert.Contains("\"type\":\"error\"", payload);
        Assert.Contains("InvalidOperationException", payload);
        Assert.Contains("\"environment\":\"test\"", payload);
        Assert.Contains("\"release\":\"abc123\"", payload);
        client.Close();
    }

    [Fact]
    public void CaptureException_CapturesInnerExceptionChain()
    {
        ObservabilityContext.ResetForTests();
        var inner = new ArgumentNullException("param");
        var outer = new InvalidOperationException("outer", inner);

        var info = ObservabilityClient.ToExceptionInfo(outer);

        Assert.Equal("InvalidOperationException", info.Type);
        Assert.NotNull(info.Cause);
        Assert.Equal("ArgumentNullException", info.Cause!.Type);
    }

    [Fact]
    public async Task CaptureMessage_ScrubsPii()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);

        client.CaptureMessage("got Bearer abc.def-123 token", Level.Warning);
        await client.FlushAsync();

        var payload = stub.CapturedBodies[0];
        Assert.Contains("Bearer [redacted]", payload);
        Assert.DoesNotContain("abc.def-123", payload);
        Assert.Contains("\"level\":\"warning\"", payload);
        client.Close();
    }

    [Fact]
    public async Task BeforeSend_CanDropEvent()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub, o => o.BeforeSend = _ => null);

        var id = client.CaptureMessage("dropped");
        await client.FlushAsync();

        Assert.False(string.IsNullOrEmpty(id)); // id still returned
        Assert.Equal(0, stub.CallCount); // but nothing sent
        client.Close();
    }

    [Fact]
    public void Capture_OnUninitializedClient_ReturnsNull()
    {
        var client = new ObservabilityClient();
        Assert.Null(client.CaptureMessage("noop"));
        Assert.Null(client.CaptureException(new Exception("noop")));
    }

    [Fact]
    public async Task CaptureHandler_FiresInAdditionToTransport()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);
        var handlerCalls = 0;
        client.RegisterCaptureHandler((_, _) => handlerCalls++);

        client.CaptureMessage("both paths");
        await client.FlushAsync();

        Assert.Equal(1, handlerCalls);
        Assert.Equal(1, stub.CallCount);
        client.Close();
    }

    [Fact]
    public async Task CaptureHandler_ThrowingHandler_DoesNotBreakCapture()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);
        client.RegisterCaptureHandler((_, _) => throw new InvalidOperationException("handler boom"));

        var id = client.CaptureMessage("still delivered"); // must not throw
        await client.FlushAsync();

        Assert.False(string.IsNullOrEmpty(id));
        Assert.Equal(1, stub.CallCount);
        client.Close();
    }

    [Fact]
    public async Task Capture_MergesAmbientScope()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);
        client.SetUser(new UserContext { Id = "user-9", OrgId = "org-9" });
        client.SetTag("feature", "checkout");

        client.CaptureMessage("scoped");
        await client.FlushAsync();

        var payload = stub.CapturedBodies[0];
        using var doc = JsonDocument.Parse(payload);
        var ev = doc.RootElement.GetProperty("events")[0];
        Assert.Equal("user-9", ev.GetProperty("user").GetProperty("id").GetString());
        Assert.Equal("checkout", ev.GetProperty("tags").GetProperty("feature").GetString());
        client.Close();
    }

    [Fact]
    public async Task WireFormat_UsesCamelCaseAndSdkRuntimeNode()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);

        client.CaptureMessage("shape check");
        await client.FlushAsync();

        var payload = stub.CapturedBodies[0];
        using var doc = JsonDocument.Parse(payload);
        var ev = doc.RootElement.GetProperty("events")[0];
        Assert.True(ev.TryGetProperty("eventId", out _));
        Assert.True(ev.TryGetProperty("timestamp", out _));
        var sdk = ev.GetProperty("sdk");
        Assert.Equal("node", sdk.GetProperty("runtime").GetString());
        Assert.Equal(ObservabilityClient.SdkName, sdk.GetProperty("name").GetString());
        client.Close();
    }
}
