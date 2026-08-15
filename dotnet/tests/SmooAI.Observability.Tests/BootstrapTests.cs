using SmooAI.Observability;
using SmooAI.Observability.Otel;

namespace SmooAI.Observability.Tests;

[Collection(OtelGlobalStateCollection.Name)]
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

        // ResolveEnv falls back to these, so "no endpoint" has to mean no
        // endpoint from ANY source or the Exporting assertion below would be
        // environment-dependent.
        using var env = new ScopedEnv(
            "SMOOAI_OBSERVABILITY_ENDPOINT",
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT");
        var stderr = new StringWriter();
        var originalError = Console.Error;
        Console.SetError(stderr);
        try
        {
            // No endpoint, no auth — must still return without throwing.
            var result = await Bootstrap.Run(new BootstrapEnv { ServiceName = "svc" });

            Assert.NotNull(result);
            Assert.True(result.Installed); // bootstrap ran…
            // …but it is NOT exporting, and the result now says so. This
            // assertion is the whole point: Installed alone used to be the only
            // signal, and it reads as "everything is fine" while nothing leaves
            // the process.
            Assert.False(result.Exporting);
            Assert.Contains("NO OTLP ENDPOINT CONFIGURED", stderr.ToString(), StringComparison.Ordinal);
            Assert.Contains("SMOOAI_OBSERVABILITY_DISABLED=true", stderr.ToString(), StringComparison.Ordinal);
        }
        finally
        {
            Console.SetError(originalError);
            Bootstrap.ResetForTests();
            ObservabilitySdk.ResetForTests();
        }
    }

    /// <summary>
    /// The inverse of the no-endpoint case: with an endpoint configured the
    /// result must claim it IS exporting. Without both halves asserted, a
    /// regression that hard-codes either value passes.
    /// </summary>
    [Fact]
    public async Task Run_WithEndpoint_ReportsExporting()
    {
        Bootstrap.ResetForTests();
        ObservabilitySdk.ResetForTests();

        var stderr = new StringWriter();
        var originalError = Console.Error;
        Console.SetError(stderr);
        try
        {
            var result = await Bootstrap.Run(new BootstrapEnv
            {
                Endpoint = "https://collector.example.test",
                Token = "pre-minted",
                ServiceName = "svc",
            });

            Assert.True(result.Installed);
            Assert.True(result.Exporting);
            Assert.NotNull(result.Otel);
            Assert.DoesNotContain("NO OTLP ENDPOINT CONFIGURED", stderr.ToString(), StringComparison.Ordinal);
        }
        finally
        {
            Console.SetError(originalError);
            Bootstrap.ResetForTests();
            ObservabilitySdk.ResetForTests();
        }
    }

    /// <summary>Clears env vars for the scope of a test and restores them after.</summary>
    private sealed class ScopedEnv : IDisposable
    {
        private readonly Dictionary<string, string?> _saved = new(StringComparer.Ordinal);

        public ScopedEnv(params string[] keys)
        {
            foreach (var key in keys)
            {
                _saved[key] = Environment.GetEnvironmentVariable(key);
                Environment.SetEnvironmentVariable(key, null);
            }
        }

        public void Dispose()
        {
            foreach (var (key, value) in _saved)
            {
                Environment.SetEnvironmentVariable(key, value);
            }
        }
    }
}

/// <summary>
/// Serializes every test class that touches the process-wide
/// <c>ObservabilitySdk</c> install guard.
///
/// <para>
/// xUnit parallelizes ACROSS collections, so a class outside this one runs
/// concurrently with it — and since <c>ObservabilitySdk.ResetForTests()</c> wipes
/// a static singleton, a concurrent reset lands between another test's two
/// <c>Setup()</c> calls and its idempotency assertion fails. That was a real
/// 3-in-8 flake on <c>OtelSetupTests.Setup_IsIdempotent</c>, reproduced on a
/// clean tree before this attribute was applied. Any new class that calls
/// <c>ObservabilitySdk.ResetForTests()</c> or <c>Bootstrap.ResetForTests()</c>
/// must join this collection.
/// </para>
/// </summary>
[CollectionDefinition(Name, DisableParallelization = true)]
public class OtelGlobalStateCollection
{
    /// <summary>Collection name — referenced by every member class.</summary>
    public const string Name = "OtelGlobalState";
}
