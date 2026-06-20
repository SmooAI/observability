using System.Net;
using System.Net.Http;
using Microsoft.AspNetCore.Http;
using SmooAI.Observability;
using SmooAI.Observability.AspNetCore;

namespace SmooAI.Observability.Tests;

public class MiddlewareTests
{
    private static ObservabilityClient NewClient(StubHttpMessageHandler stub)
    {
        var client = new ObservabilityClient();
        client.Init(new ClientOptions
        {
            Dsn = "https://ingest.test/hook",
            Environment = "test",
            HttpClient = new HttpClient(stub),
        });
        return client;
    }

    private static DefaultHttpContext NewContext()
    {
        var ctx = new DefaultHttpContext();
        ctx.Request.Method = "GET";
        ctx.Request.Path = "/widgets";
        ctx.Request.QueryString = new QueryString("?id=1");
        ctx.Request.Headers["Authorization"] = "Bearer secret-token";
        return ctx;
    }

    [Fact]
    public async Task Middleware_CapturesUnhandledExceptionAndRethrows()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);
        var middleware = new SmooObservabilityMiddleware(
            _ => throw new InvalidOperationException("handler blew up"),
            client);

        await Assert.ThrowsAsync<InvalidOperationException>(() => middleware.InvokeAsync(NewContext()));

        await client.FlushAsync();
        Assert.Equal(1, stub.CallCount);
        var payload = stub.CapturedBodies[0];
        Assert.Contains("InvalidOperationException", payload);
        Assert.Contains("\"http.method\":\"GET\"", payload);
        client.Close();
    }

    [Fact]
    public async Task Middleware_AttachesScrubbedRequestBreadcrumb()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);
        var middleware = new SmooObservabilityMiddleware(
            _ => throw new InvalidOperationException("boom"),
            client);

        await Assert.ThrowsAsync<InvalidOperationException>(() => middleware.InvokeAsync(NewContext()));
        await client.FlushAsync();

        var payload = stub.CapturedBodies[0];
        // The Authorization header value must be redacted in the breadcrumb.
        Assert.DoesNotContain("secret-token", payload);
        Assert.Contains("\"category\":\"http\"", payload);
        client.Close();
    }

    [Fact]
    public async Task Middleware_PassesThroughOnSuccess()
    {
        ObservabilityContext.ResetForTests();
        var stub = StubHttpMessageHandler.AlwaysStatus(HttpStatusCode.OK);
        var client = NewClient(stub);
        var called = false;
        var middleware = new SmooObservabilityMiddleware(
            _ => { called = true; return Task.CompletedTask; },
            client);

        await middleware.InvokeAsync(NewContext());

        Assert.True(called);
        Assert.Equal(0, stub.CallCount); // no exception -> nothing captured
        client.Close();
    }
}
