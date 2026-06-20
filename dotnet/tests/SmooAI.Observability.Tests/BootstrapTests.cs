using SmooAI.Observability;
using SmooAI.Observability.Otel;

namespace SmooAI.Observability.Tests;

[Collection("Bootstrap")]
public class BootstrapTests
{
    [Fact]
    public async Task Run_Disabled_DoesNotInstall()
    {
        Bootstrap.ResetForTests();
        ObservabilitySdk.ResetForTests();

        var result = await Bootstrap.Run(new BootstrapEnv { Disabled = true });

        Assert.False(result.Installed);
        Assert.Null(result.Otel);
        Bootstrap.ResetForTests();
    }

    [Fact]
    public async Task Run_IsIdempotent()
    {
        Bootstrap.ResetForTests();
        ObservabilitySdk.ResetForTests();

        var first = await Bootstrap.Run(new BootstrapEnv
        {
            Endpoint = "https://ingest.test",
            Token = "pre-minted",
            ServiceName = "svc",
            Environment = "test",
        });
        var second = await Bootstrap.Run(new BootstrapEnv { Disabled = true });

        Assert.Same(first, second); // second call returns the cached result
        Assert.True(first.Installed);
        Bootstrap.ResetForTests();
        ObservabilitySdk.ResetForTests();
    }

    [Fact]
    public async Task Run_NeverThrows_OnBadConfig()
    {
        Bootstrap.ResetForTests();
        ObservabilitySdk.ResetForTests();

        // No endpoint, no auth — must still return without throwing.
        var result = await Bootstrap.Run(new BootstrapEnv { ServiceName = "svc" });

        Assert.NotNull(result);
        Bootstrap.ResetForTests();
        ObservabilitySdk.ResetForTests();
    }
}

[CollectionDefinition("Bootstrap", DisableParallelization = true)]
public class BootstrapCollection { }
