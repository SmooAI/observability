using System.Diagnostics;
using SmooAI.Observability.Otel;

namespace SmooAI.Observability.Tests;

/// <summary>
/// Verifies the opt-in ASP.NET Core + HttpClient auto-instrumentation wiring in
/// <see cref="ObservabilitySdk.Setup"/>. We assert on subscription rather than on
/// emitted spans: an <see cref="ActivitySource"/> reports
/// <see cref="ActivitySource.HasListeners"/> == true only when a tracer provider
/// is subscribed to it, so it's a reliable proxy for "the instrumentation
/// registered its source".
/// </summary>
public class OtelInstrumentationTests
{
    // Source names the OTel instrumentation packages subscribe to. Stable across
    // the 1.x line: the HttpClient instrumentation listens to System.Net.Http and
    // the ASP.NET Core instrumentation listens to Microsoft.AspNetCore.
    private const string HttpClientSourceName = "System.Net.Http";
    private const string AspNetCoreSourceName = "Microsoft.AspNetCore";

    [Fact]
    public void Default_DoesNotSubscribeToInstrumentationSources()
    {
        ObservabilitySdk.ResetForTests();
        using var http = new ActivitySource(HttpClientSourceName);
        using var aspnet = new ActivitySource(AspNetCoreSourceName);

        // Sanity: no listeners before setup.
        Assert.False(http.HasListeners());
        Assert.False(aspnet.HasListeners());

        var handle = ObservabilitySdk.Setup(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpTracesEndpoint = "https://ingest.test/v1/traces",
            // EnableAspNetCoreInstrumentation / EnableHttpInstrumentation default to false.
        });

        Assert.False(http.HasListeners());
        Assert.False(aspnet.HasListeners());

        handle.Dispose();
        ObservabilitySdk.ResetForTests();
    }

    [Fact]
    public void EnableHttpInstrumentation_SubscribesToHttpClientSource()
    {
        ObservabilitySdk.ResetForTests();
        using var http = new ActivitySource(HttpClientSourceName);
        using var aspnet = new ActivitySource(AspNetCoreSourceName);

        var handle = ObservabilitySdk.Setup(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpTracesEndpoint = "https://ingest.test/v1/traces",
            EnableHttpInstrumentation = true,
        });

        Assert.True(http.HasListeners());
        // Only HTTP requested — ASP.NET Core stays off.
        Assert.False(aspnet.HasListeners());

        handle.Dispose();
        ObservabilitySdk.ResetForTests();
    }

    [Fact]
    public void EnableAspNetCoreInstrumentation_SubscribesToAspNetCoreSource()
    {
        ObservabilitySdk.ResetForTests();
        using var http = new ActivitySource(HttpClientSourceName);
        using var aspnet = new ActivitySource(AspNetCoreSourceName);

        var handle = ObservabilitySdk.Setup(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpTracesEndpoint = "https://ingest.test/v1/traces",
            EnableAspNetCoreInstrumentation = true,
        });

        Assert.True(aspnet.HasListeners());
        // Only ASP.NET Core requested — HTTP client stays off.
        Assert.False(http.HasListeners());

        handle.Dispose();
        ObservabilitySdk.ResetForTests();
    }

    [Fact]
    public void EnableBoth_SubscribesToBothSources()
    {
        ObservabilitySdk.ResetForTests();
        using var http = new ActivitySource(HttpClientSourceName);
        using var aspnet = new ActivitySource(AspNetCoreSourceName);

        var handle = ObservabilitySdk.Setup(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpTracesEndpoint = "https://ingest.test/v1/traces",
            OtlpMetricsEndpoint = "https://ingest.test/v1/metrics",
            EnableAspNetCoreInstrumentation = true,
            EnableHttpInstrumentation = true,
        });

        Assert.True(http.HasListeners());
        Assert.True(aspnet.HasListeners());

        handle.Dispose();
        ObservabilitySdk.ResetForTests();
    }

    [Fact]
    public void Enable_WithoutTracesEndpoint_DoesNotBuildTracerOrSubscribe()
    {
        ObservabilitySdk.ResetForTests();
        using var http = new ActivitySource(HttpClientSourceName);
        using var aspnet = new ActivitySource(AspNetCoreSourceName);

        // No traces endpoint -> no tracer provider -> instrumentation cannot subscribe.
        var handle = ObservabilitySdk.Setup(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpMetricsEndpoint = "https://ingest.test/v1/metrics",
            EnableAspNetCoreInstrumentation = true,
            EnableHttpInstrumentation = true,
        });

        Assert.False(http.HasListeners());
        Assert.False(aspnet.HasListeners());

        handle.Dispose();
        ObservabilitySdk.ResetForTests();
    }
}
