using System.Collections.Concurrent;
using System.Diagnostics;
using System.Diagnostics.Metrics;

namespace SmooAI.Observability.Metrics;

/// <summary>
/// Thin Smoo-flavored API over <see cref="System.Diagnostics.Metrics"/> — the
/// .NET analogue of the TS <c>getMetricsClient</c> wrapper over the OTel Meter
/// API. Instruments are cached by <c>(meterName, instrumentName)</c> so handles
/// don't leak. Every method is error-safe (observability must not throw).
///
/// The <see cref="Meter"/> name is what OTel exports as
/// <c>instrumentation.scope.name</c>, so pass your service name to group metrics
/// in dashboards. The <see cref="Otel.ObservabilitySdk"/> registers the default
/// meter name as a metric source so these are exported.
/// </summary>
public sealed class MetricsClient
{
    /// <summary>Default meter name when none is supplied.</summary>
    public const string DefaultMeterName = "@smooai/observability";

    private static readonly ConcurrentDictionary<string, Meter> Meters = new();
    private static readonly ConcurrentDictionary<string, Counter<long>> Counters = new();
    private static readonly ConcurrentDictionary<string, Histogram<double>> Histograms = new();

    private readonly string _meterName;

    private MetricsClient(string meterName) => _meterName = meterName;

    /// <summary>
    /// Build a metrics client bound to a meter. Defaults to
    /// <see cref="DefaultMeterName"/> so any caller can emit with no args.
    /// </summary>
    public static MetricsClient Get(string meterName = DefaultMeterName) =>
        new(string.IsNullOrEmpty(meterName) ? DefaultMeterName : meterName);

    /// <summary>Names of all meters created so far — for OTel source registration.</summary>
    public static IReadOnlyCollection<string> KnownMeterNames => Meters.Keys.ToArray();

    /// <summary>Add to a monotonically-increasing counter.</summary>
    public void Counter(string name, long value = 1, IReadOnlyDictionary<string, string>? attrs = null)
    {
        try
        {
            GetCounter(name).Add(value, ToTags(attrs));
        }
        catch
        {
            // swallow
        }
    }

    /// <summary>Record a histogram observation (latencies, sizes, ...).</summary>
    public void Histogram(string name, double value, IReadOnlyDictionary<string, string>? attrs = null)
    {
        try
        {
            GetHistogram(name).Record(value, ToTags(attrs));
        }
        catch
        {
            // swallow
        }
    }

    /// <summary>Record a duration histogram (ms unit), rendering as a duration.</summary>
    public void Timing(string name, double ms, IReadOnlyDictionary<string, string>? attrs = null)
    {
        try
        {
            GetHistogram(name, "ms").Record(ms, ToTags(attrs));
        }
        catch
        {
            // swallow
        }
    }

    /// <summary>
    /// Start a wall-clock timer. Dispose (or call the returned action) when the
    /// operation completes to record elapsed ms as a timing histogram.
    /// </summary>
    public IDisposable StartTimer(string name, IReadOnlyDictionary<string, string>? attrs = null) =>
        new TimerScope(this, name, attrs);

    /// <summary>
    /// Wrap an async function in a timing measurement. Records elapsed ms with a
    /// <c>status=success|error</c> attribute and rethrows on failure.
    /// </summary>
    public async Task<T> WithTimingAsync<T>(string name, Func<Task<T>> fn, IReadOnlyDictionary<string, string>? attrs = null)
    {
        ArgumentNullException.ThrowIfNull(fn);
        var sw = Stopwatch.StartNew();
        try
        {
            var result = await fn().ConfigureAwait(false);
            Timing(name, sw.Elapsed.TotalMilliseconds, WithStatus(attrs, "success"));
            return result;
        }
        catch
        {
            Timing(name, sw.Elapsed.TotalMilliseconds, WithStatus(attrs, "error"));
            throw;
        }
    }

    private Counter<long> GetCounter(string name) =>
        Counters.GetOrAdd($"{_meterName}::{name}", _ => GetMeter().CreateCounter<long>(name));

    private Histogram<double> GetHistogram(string name, string? unit = null) =>
        Histograms.GetOrAdd($"{_meterName}::{name}::{unit}", _ => GetMeter().CreateHistogram<double>(name, unit));

    private Meter GetMeter() => Meters.GetOrAdd(_meterName, n => new Meter(n));

    private static KeyValuePair<string, object?>[] ToTags(IReadOnlyDictionary<string, string>? attrs)
    {
        if (attrs is null || attrs.Count == 0)
        {
            return Array.Empty<KeyValuePair<string, object?>>();
        }
        var tags = new KeyValuePair<string, object?>[attrs.Count];
        var i = 0;
        foreach (var (k, v) in attrs)
        {
            tags[i++] = new KeyValuePair<string, object?>(k, v);
        }
        return tags;
    }

    private static Dictionary<string, string> WithStatus(IReadOnlyDictionary<string, string>? attrs, string status)
    {
        var merged = attrs is null
            ? new Dictionary<string, string>(StringComparer.Ordinal)
            : new Dictionary<string, string>(attrs.Count + 1, StringComparer.Ordinal);
        if (attrs is not null)
        {
            foreach (var (k, v) in attrs)
            {
                merged[k] = v;
            }
        }
        merged["status"] = status;
        return merged;
    }

    /// <summary>Test seam — drop cached instruments + meters.</summary>
    internal static void ResetForTests()
    {
        foreach (var meter in Meters.Values)
        {
            meter.Dispose();
        }
        Meters.Clear();
        Counters.Clear();
        Histograms.Clear();
    }

    private sealed class TimerScope : IDisposable
    {
        private readonly MetricsClient _client;
        private readonly string _name;
        private readonly IReadOnlyDictionary<string, string>? _attrs;
        private readonly Stopwatch _stopwatch;
        private bool _stopped;

        public TimerScope(MetricsClient client, string name, IReadOnlyDictionary<string, string>? attrs)
        {
            _client = client;
            _name = name;
            _attrs = attrs;
            _stopwatch = Stopwatch.StartNew();
        }

        public void Dispose()
        {
            if (_stopped)
            {
                return;
            }
            _stopped = true;
            _client.Timing(_name, _stopwatch.Elapsed.TotalMilliseconds, _attrs);
        }
    }
}
