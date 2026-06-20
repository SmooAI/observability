using System.Net.Http;
using OpenTelemetry;
using OpenTelemetry.Exporter;
using OpenTelemetry.Metrics;
using OpenTelemetry.Resources;
using OpenTelemetry.Trace;
using SmooAI.Observability.Auth;
using SmooAI.Observability.Metrics;

namespace SmooAI.Observability.Otel;

/// <summary>
/// Options for <see cref="ObservabilitySdk.Setup"/>. Port of the TS
/// <c>SetupOtelOptions</c>.
/// </summary>
public sealed class SetupOtelOptions
{
    /// <summary>Service name surfaced in spans/metrics (e.g. 'smoo-backend').</summary>
    public string ServiceName { get; set; } = "smoo-service";

    /// <summary>OTLP/HTTP endpoint for traces. Full URL incl. <c>/v1/traces</c>.</summary>
    public string? OtlpTracesEndpoint { get; set; }

    /// <summary>OTLP/HTTP endpoint for metrics. Full URL incl. <c>/v1/metrics</c>.</summary>
    public string? OtlpMetricsEndpoint { get; set; }

    /// <summary>Deployment environment string ('production', 'staging', 'dev', 'local').</summary>
    public string? Environment { get; set; }

    /// <summary>Release identifier — git sha, deployment version, package version.</summary>
    public string? Release { get; set; }

    /// <summary>
    /// When set, traces + metrics export with a fresh Bearer token pulled from
    /// this provider on every request (via <see cref="AuthHeaderHandler"/>) —
    /// no header snapshot, no expiry drift.
    /// </summary>
    public TokenProvider? TokenProvider { get; set; }

    /// <summary>Static headers merged onto every OTLP request (e.g. user-agent).</summary>
    public IReadOnlyDictionary<string, string>? OtlpHeaders { get; set; }

    /// <summary>Metric export interval in ms. Default 30000 (30s).</summary>
    public int MetricExportIntervalMs { get; set; } = 30_000;

    /// <summary>
    /// Additional <see cref="System.Diagnostics.ActivitySource"/> names to collect
    /// traces from, beyond the SDK's own tracer.
    /// </summary>
    public IReadOnlyList<string>? AdditionalActivitySources { get; set; }

    /// <summary>
    /// Additional <see cref="System.Diagnostics.Metrics.Meter"/> names to collect
    /// metrics from, beyond the SDK's default meter.
    /// </summary>
    public IReadOnlyList<string>? AdditionalMeterNames { get; set; }
}

/// <summary>
/// Handle returned by <see cref="ObservabilitySdk.Setup"/> — flush / shutdown
/// hooks the host wires into SIGTERM / shutdown. Port of the TS
/// <c>OtelSdkHandle</c>.
/// </summary>
public sealed class OtelSdkHandle : IDisposable
{
    private readonly TracerProvider? _tracerProvider;
    private readonly MeterProvider? _meterProvider;
    private bool _disposed;

    internal OtelSdkHandle(TracerProvider? tracerProvider, MeterProvider? meterProvider)
    {
        _tracerProvider = tracerProvider;
        _meterProvider = meterProvider;
    }

    /// <summary>The SDK's own <see cref="System.Diagnostics.ActivitySource"/> name.</summary>
    public const string ActivitySourceName = "smooai.observability";

    /// <summary>
    /// Force-flush traces + metrics. Best-effort, bounded by
    /// <paramref name="timeoutMs"/>. Never throws.
    /// </summary>
    public void Flush(int timeoutMs = 2000)
    {
        try
        {
            _tracerProvider?.ForceFlush(timeoutMs);
        }
        catch
        {
            // swallow
        }
        try
        {
            _meterProvider?.ForceFlush(timeoutMs);
        }
        catch
        {
            // swallow
        }
    }

    /// <inheritdoc />
    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        Flush();
        _tracerProvider?.Dispose();
        _meterProvider?.Dispose();
    }
}

/// <summary>
/// One-call OpenTelemetry bootstrap for traces + metrics, exporting via OTLP/HTTP
/// to the Smoo ingest endpoint. Idempotent — calling <see cref="Setup"/> twice
/// returns the first handle. Port of the TS <c>setupOtelSdk</c>.
/// </summary>
public static class ObservabilitySdk
{
    private static readonly object Gate = new();
    private static OtelSdkHandle? _installed;

    /// <summary>The SDK's tracer/activity-source name.</summary>
    public const string TracerName = OtelSdkHandle.ActivitySourceName;

    /// <summary>
    /// Initialize the OTel SDK. Idempotent. Never throws — on any failure it
    /// returns a handle with no providers so the host keeps running.
    /// </summary>
    public static OtelSdkHandle Setup(SetupOtelOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        lock (Gate)
        {
            if (_installed is not null)
            {
                return _installed;
            }

            try
            {
                _installed = Build(options);
            }
            catch
            {
                // Construction failed — return an inert handle.
                _installed = new OtelSdkHandle(null, null);
            }
            return _installed;
        }
    }

    private static OtelSdkHandle Build(SetupOtelOptions options)
    {
        var resourceBuilder = ResourceBuilder.CreateDefault().AddService(options.ServiceName);
        var resourceAttrs = new List<KeyValuePair<string, object>>();
        if (!string.IsNullOrEmpty(options.Release))
        {
            resourceAttrs.Add(new KeyValuePair<string, object>("service.version", options.Release!));
        }
        if (!string.IsNullOrEmpty(options.Environment))
        {
            resourceAttrs.Add(new KeyValuePair<string, object>("deployment.environment.name", options.Environment!));
        }
        if (resourceAttrs.Count > 0)
        {
            resourceBuilder.AddAttributes(resourceAttrs);
        }

        TracerProvider? tracerProvider = null;
        MeterProvider? meterProvider = null;

        // Traces.
        if (!string.IsNullOrEmpty(options.OtlpTracesEndpoint))
        {
            var tracerBuilder = OpenTelemetry.Sdk.CreateTracerProviderBuilder()
                .SetResourceBuilder(resourceBuilder)
                .AddSource(TracerName);
            if (options.AdditionalActivitySources is { Count: > 0 })
            {
                tracerBuilder.AddSource(options.AdditionalActivitySources.ToArray());
            }
            tracerBuilder.AddOtlpExporter(o => ConfigureExporter(o, options.OtlpTracesEndpoint!, options));
            tracerProvider = tracerBuilder.Build();
        }

        // Metrics.
        if (!string.IsNullOrEmpty(options.OtlpMetricsEndpoint))
        {
            var meterBuilder = OpenTelemetry.Sdk.CreateMeterProviderBuilder()
                .SetResourceBuilder(resourceBuilder)
                .AddMeter(MetricsClient.DefaultMeterName);
            if (options.AdditionalMeterNames is { Count: > 0 })
            {
                meterBuilder.AddMeter(options.AdditionalMeterNames.ToArray());
            }
            meterBuilder.AddOtlpExporter((exporterOptions, readerOptions) =>
            {
                ConfigureExporter(exporterOptions, options.OtlpMetricsEndpoint!, options);
                readerOptions.PeriodicExportingMetricReaderOptions.ExportIntervalMilliseconds = options.MetricExportIntervalMs;
            });
            meterProvider = meterBuilder.Build();
        }

        return new OtelSdkHandle(tracerProvider, meterProvider);
    }

    private static void ConfigureExporter(OtlpExporterOptions exporterOptions, string endpoint, SetupOtelOptions options)
    {
        exporterOptions.Endpoint = new Uri(endpoint);
        exporterOptions.Protocol = OtlpExportProtocol.HttpProtobuf;

        if (options.OtlpHeaders is { Count: > 0 })
        {
            // OTLP header format is "k1=v1,k2=v2".
            exporterOptions.Headers = string.Join(",", options.OtlpHeaders.Select(kv => $"{kv.Key}={kv.Value}"));
        }

        if (options.TokenProvider is not null)
        {
            var tokenProvider = options.TokenProvider;
            exporterOptions.HttpClientFactory = () => new HttpClient(new AuthHeaderHandler(tokenProvider));
        }
    }

    /// <summary>Test seam — wipes the install guard so the next call re-initializes.</summary>
    internal static void ResetForTests()
    {
        lock (Gate)
        {
            _installed?.Dispose();
            _installed = null;
        }
    }
}
