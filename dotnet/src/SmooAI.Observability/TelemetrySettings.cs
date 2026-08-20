using System.Globalization;
using System.Text.Json;

namespace SmooAI.Observability;

/// <summary>Resolved telemetry settings.</summary>
/// <param name="Enabled">Kill switch. When false nothing is emitted, errors included.</param>
/// <param name="BrowserLogSamplingRatio">
/// Session-scoped browser log sampling ratio. Applied ONCE per session (or
/// inherited from the trace where one exists) — never per line.
/// </param>
/// <param name="MinimumLogLevel">Minimum level to emit.</param>
/// <param name="TraceSamplingRatio">Head-based trace sampling ratio. ADR-010 default: TraceIdRatioBased(0.1).</param>
public readonly record struct TelemetrySettings(
    bool Enabled,
    double BrowserLogSamplingRatio,
    CanonicalLevel MinimumLogLevel,
    double TraceSamplingRatio);

/// <summary>
/// ADR-097 W1 — config-served telemetry settings. .NET port of
/// <c>packages/core/src/telemetry-settings.ts</c>, pinned by
/// <c>parity/sampling-corpus.json</c>.
/// </summary>
/// <remarks>
/// <para>
/// These are <c>@smooai/config</c> <b>public-tier</b>, org-scoped keys. Public
/// tier is mandatory: a browser can never be served secret tier (ADR-075), and
/// the whole point is that changing a key changes every client's behaviour on
/// its next config read. <b>No secret may ever enter this key set.</b>
/// </para>
/// <para>
/// FAIL-SAFE IS THE POINT. Unreachable provider, malformed payload,
/// out-of-range value → the compiled-in ADR-010 defaults. Never "sample
/// everything out": a telemetry system that goes silent when its config server
/// hiccups is worse than useless. A caller whose config read <em>failed</em>
/// passes <c>default</c> and gets exactly those defaults — which is why this
/// method has no error channel.
/// </para>
/// </remarks>
public static class TelemetrySettingsResolver
{
    /// <summary>Boolean kill switch. false disables ALL telemetry emission.</summary>
    public const string KeyEnabled = "observabilityEnabled";

    /// <summary>Number 0.0–1.0 — session-scoped browser log sampling ratio.</summary>
    public const string KeyBrowserLogSamplingRatio = "observabilityBrowserLogSamplingRatio";

    /// <summary>Minimum log level to emit (TRACE|DEBUG|INFO|WARN|ERROR|FATAL).</summary>
    public const string KeyMinimumLogLevel = "observabilityMinimumLogLevel";

    /// <summary>Number 0.0–1.0 — head-based trace sampling ratio.</summary>
    public const string KeyTraceSamplingRatio = "observabilityTraceSamplingRatio";

    /// <summary>Compiled-in ADR-010 defaults. Every failure path lands here.</summary>
    public static readonly TelemetrySettings Defaults = new(true, 1.0, CanonicalLevel.Info, 0.1);

    /// <summary>
    /// Ratio coercion: a finite number, or a decimal numeric string (public
    /// config often round-trips values as strings), clamped into [0, 1] — an
    /// operator who writes 1.5 means "all". Anything else (missing, NaN,
    /// Infinity, boolean, object, unparseable string) → the compiled-in default,
    /// never 0.
    /// </summary>
    /// <remarks>
    /// The asymmetry is deliberate: a <em>malformed</em> value falls back, a
    /// <em>valid but out-of-range</em> value is clamped. -1 clamps to 0
    /// (telemetry off) because that is an explicit operator value, and 0 is
    /// settable anyway.
    /// </remarks>
    private static double CoerceRatio(JsonElement? raw, double fallback)
    {
        double n;
        switch (raw?.ValueKind)
        {
            case JsonValueKind.Number when raw.Value.TryGetDouble(out var d):
                n = d;
                break;
            case JsonValueKind.String:
                var s = raw.Value.GetString()?.Trim();
                if (string.IsNullOrEmpty(s) || !double.TryParse(s, NumberStyles.Float, CultureInfo.InvariantCulture, out n))
                {
                    return fallback;
                }

                break;
            default:
                return fallback;
        }

        if (double.IsNaN(n) || double.IsInfinity(n))
        {
            return fallback;
        }

        return Math.Min(1.0, Math.Max(0.0, n));
    }

    private static bool CoerceBoolean(JsonElement? raw, bool fallback) => raw?.ValueKind switch
    {
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        JsonValueKind.String => raw.Value.GetString()?.Trim().ToLowerInvariant() switch
        {
            "true" => true,
            "false" => false,
            _ => fallback,
        },
        _ => fallback,
    };

    // Sampling.ParseLevel (not NormalizeLevel) on purpose: normalize maps
    // unknown spellings to INFO, which is right for an incoming log line but
    // wrong here — a typo'd config value must fall back to the default, not
    // silently reset the floor.
    private static CanonicalLevel CoerceLevel(JsonElement? raw, CanonicalLevel fallback) =>
        raw?.ValueKind == JsonValueKind.String && Sampling.ParseLevel(raw.Value.GetString()!) is { } parsed ? parsed : fallback;

    /// <summary>
    /// Turn a raw config payload into settings. Total method — never throws,
    /// always returns a usable value. Unknown/extra keys are ignored; anything
    /// that is not a JSON object (null, a scalar, an array, or an unset
    /// <see cref="JsonElement"/>) resolves to <see cref="Defaults"/>.
    /// </summary>
    public static TelemetrySettings Resolve(JsonElement raw)
    {
        if (raw.ValueKind != JsonValueKind.Object)
        {
            return Defaults;
        }

        static JsonElement? Get(JsonElement bag, string key) => bag.TryGetProperty(key, out var v) ? v : null;

        return new TelemetrySettings(
            CoerceBoolean(Get(raw, KeyEnabled), Defaults.Enabled),
            CoerceRatio(Get(raw, KeyBrowserLogSamplingRatio), Defaults.BrowserLogSamplingRatio),
            CoerceLevel(Get(raw, KeyMinimumLogLevel), Defaults.MinimumLogLevel),
            CoerceRatio(Get(raw, KeyTraceSamplingRatio), Defaults.TraceSamplingRatio));
    }
}
