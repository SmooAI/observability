using SmooAI.Observability.Otel;

namespace SmooAI.Observability.Tests;

public class OtelSetupTests
{
    [Fact]
    public void Setup_IsIdempotent()
    {
        ObservabilitySdk.ResetForTests();
        var first = ObservabilitySdk.Setup(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpTracesEndpoint = "https://ingest.test/v1/traces",
        });
        var second = ObservabilitySdk.Setup(new SetupOtelOptions { ServiceName = "other" });

        Assert.Same(first, second);
        ObservabilitySdk.ResetForTests();
    }

    [Fact]
    public void Setup_WithNoEndpoints_StillReturnsHandle()
    {
        ObservabilitySdk.ResetForTests();
        var handle = ObservabilitySdk.Setup(new SetupOtelOptions { ServiceName = "svc" });
        Assert.NotNull(handle);
        // Flush/Dispose must be safe even with no providers.
        handle.Flush();
        handle.Dispose();
        ObservabilitySdk.ResetForTests();
    }

    [Fact]
    public void Setup_BuildsProvidersWhenEndpointsGiven()
    {
        ObservabilitySdk.ResetForTests();
        var handle = ObservabilitySdk.Setup(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpTracesEndpoint = "https://ingest.test/v1/traces",
            OtlpMetricsEndpoint = "https://ingest.test/v1/metrics",
            Environment = "test",
            Release = "rel-1",
        });

        Assert.NotNull(handle);
        handle.Flush(500);
        ObservabilitySdk.ResetForTests();
    }
}
