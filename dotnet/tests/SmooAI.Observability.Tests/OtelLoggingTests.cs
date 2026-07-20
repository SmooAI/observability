using System.Diagnostics;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using OpenTelemetry.Logs;
using SmooAI.Observability.Otel;

namespace SmooAI.Observability.Tests;

/// <summary>
/// Verifies the logs signal wired by <see cref="SmooObservabilityLoggingExtensions"/>:
/// records emitted inside an active <see cref="Activity"/> carry that span's W3C
/// trace/span id, and the extension is a no-op when no logs endpoint is configured.
/// </summary>
public class OtelLoggingTests
{
    private static bool HasOtelProvider(IServiceCollection services)
    {
        using var sp = services.BuildServiceProvider();
        return sp.GetServices<ILoggerProvider>()
            .Any(p => p.GetType().Namespace?.StartsWith("OpenTelemetry", StringComparison.Ordinal) == true);
    }

    [Fact]
    public void NoLogsEndpoint_IsNoOp_NoOtelProviderRegistered()
    {
        var services = new ServiceCollection();
        services.AddLogging(b => b.AddSmooObservability(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpLogsEndpoint = null,
        }));

        Assert.False(HasOtelProvider(services));
    }

    [Fact]
    public void LogsEndpoint_RegistersOtelProvider()
    {
        var services = new ServiceCollection();
        services.AddLogging(b => b.AddSmooObservability(new SetupOtelOptions
        {
            ServiceName = "svc",
            OtlpLogsEndpoint = "https://ingest.test/v1/logs",
        }));

        Assert.True(HasOtelProvider(services));
    }

    [Fact]
    public void Disabled_Env_IsNoOp()
    {
        var services = new ServiceCollection();
        services.AddLogging(b => b.AddSmooObservability(new BootstrapEnv
        {
            Endpoint = "https://ingest.test",
            Disabled = true,
        }));

        Assert.False(HasOtelProvider(services));
    }

    [Fact]
    public void LogWithinActiveActivity_CarriesRealTraceAndSpanId()
    {
        using var source = new ActivitySource("smooai.observability.logtests");
        using var listener = new ActivityListener
        {
            ShouldListenTo = s => s.Name == "smooai.observability.logtests",
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllData,
        };
        ActivitySource.AddActivityListener(listener);

        var records = new List<LogRecord>();
        using var factory = LoggerFactory.Create(b =>
        {
            // The extension wires the OTel logging provider (OTLP → fake endpoint,
            // never reached); capture the same records in-memory to assert on them.
            b.AddSmooObservability(new SetupOtelOptions
            {
                ServiceName = "svc",
                OtlpLogsEndpoint = "http://localhost:1/v1/logs",
            });
            b.Services.Configure<OpenTelemetryLoggerOptions>(o => o.AddInMemoryExporter(records));
        });

        var logger = factory.CreateLogger("test");
        using (var activity = source.StartActivity("op"))
        {
            Assert.NotNull(activity);
            logger.LogInformation("inside span");

            var record = Assert.Single(records);
            Assert.Equal(activity!.TraceId, record.TraceId);
            Assert.Equal(activity.SpanId, record.SpanId);
        }
    }
}
