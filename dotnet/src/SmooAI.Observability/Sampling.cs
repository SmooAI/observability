namespace SmooAI.Observability;

/// <summary>
/// ADR-096 canonical log level, uppercase.
/// </summary>
/// <remarks>
/// Distinct from <see cref="Level"/>, which is the lowercase <em>event wire</em>
/// severity on the ingest envelope. This is the log-line severity the ClickHouse
/// queries filter on, and <c>level IN ('ERROR','FATAL')</c> is CASE-SENSITIVE —
/// emitting <c>"error"</c> silently makes every error a non-error.
/// </remarks>
public enum CanonicalLevel
{
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

/// <summary>Input to <see cref="Sampling.ShouldEmitLog"/>.</summary>
/// <param name="Level">Level as emitted by the caller; normalized internally.</param>
/// <param name="SessionId">Stable per-page session id, used when no trace exists.</param>
/// <param name="Enabled">Kill switch — false disables all emission, errors included.</param>
/// <param name="MinimumLevel">Minimum level to emit.</param>
/// <param name="LogSamplingRatio">Session-scoped browser log sampling ratio.</param>
/// <param name="TraceSampled">
/// The trace's own sampling decision, when a trace context exists (null when it
/// does not). Where a trace exists its decision WINS, so spans and logs never
/// disagree about whether a request was recorded.
/// </param>
public readonly record struct LogSamplingInput(
    string Level,
    string SessionId,
    bool Enabled,
    CanonicalLevel MinimumLevel,
    double LogSamplingRatio,
    bool? TraceSampled = null);

/// <summary>
/// ADR-097 — session-scoped sampling. .NET port of
/// <c>packages/core/src/sampling.ts</c>.
/// </summary>
/// <remarks>
/// <para>
/// THE CORE RULE: the sampling decision is made ONCE per session (or per trace,
/// where one exists) and applies to EVERY log line under it. The invariant this
/// buys: any trace you can open has 100% of its log lines. Never a partial view.
/// </para>
/// <para>
/// Every vector this class must reproduce lives in
/// <c>parity/sampling-corpus.json</c> and is asserted by
/// <c>ParityCorpusTests</c> — the same file the TS, Rust, Python and Go lanes
/// load. See <c>parity/README.md</c> for the hash derivation and the porting
/// traps.
/// </para>
/// </remarks>
public static class Sampling
{
    private const uint FnvOffsetBasis32 = 0x811c9dc5;
    private const uint FnvPrime32 = 0x01000193;

    // 2^32 as an exact double — dividing a uint by this is a pure exponent
    // adjustment, so every language gets the identical double.
    private const double TwoPow32 = 4294967296.0;

    /// <summary>
    /// FNV-1a 32-bit over the UTF-8 bytes of <paramref name="value"/>.
    /// </summary>
    /// <remarks>
    /// XOR-then-multiply (this is FNV-<em>1a</em>, not FNV-1). <c>uint</c>
    /// arithmetic is unchecked and wraps natively; the result must NOT be routed
    /// through <c>int</c>, which would make hashes above 2^31 negative and flip
    /// their sampling decision.
    /// </remarks>
    public static uint Fnv1a32(string value)
    {
        ArgumentNullException.ThrowIfNull(value);

        var h = FnvOffsetBasis32;
        foreach (var b in System.Text.Encoding.UTF8.GetBytes(value))
        {
            h ^= b;
            h *= FnvPrime32;
        }

        return h;
    }

    /// <summary>
    /// The one sampling primitive. Deterministic and stable for the lifetime of
    /// an id.
    /// </summary>
    /// <remarks>
    /// <list type="bullet">
    /// <item>non-finite <paramref name="ratio"/> → IN (fail open — telemetry
    /// going dark on a config hiccup is the failure ADR-097 forbids;
    /// <c>x &lt; NaN</c> is false everywhere, so the naive path would sample
    /// <em>everything</em> out)</item>
    /// <item><c>ratio &lt;= 0.0</c> → OUT, <c>ratio &gt;= 1.0</c> → IN, both
    /// taken before any float math so 1.0 can never drop and 0.0 can never
    /// keep</item>
    /// <item>otherwise <c>(hash / 2^32) &lt; ratio</c>, strict less-than</item>
    /// </list>
    /// </remarks>
    /// <param name="id">Session id, or trace id where a trace exists.</param>
    /// <param name="ratio">0.0 (never) .. 1.0 (always).</param>
    public static bool SampleDecision(string id, double ratio)
    {
        if (double.IsNaN(ratio) || double.IsInfinity(ratio))
        {
            return true;
        }

        if (ratio <= 0.0)
        {
            return false;
        }

        if (ratio >= 1.0)
        {
            return true;
        }

        return Fnv1a32(id) / TwoPow32 < ratio;
    }

    /// <summary>The canonical uppercase spelling written to the wire.</summary>
    public static string ToWire(this CanonicalLevel level) => level switch
    {
        CanonicalLevel.Trace => "TRACE",
        CanonicalLevel.Debug => "DEBUG",
        CanonicalLevel.Info => "INFO",
        CanonicalLevel.Warn => "WARN",
        CanonicalLevel.Error => "ERROR",
        CanonicalLevel.Fatal => "FATAL",
        _ => "INFO",
    };

    // Ordering used by the minimum-level filter. Matches OTel severity numbers.
    private static int Rank(CanonicalLevel level) => level switch
    {
        CanonicalLevel.Trace => 1,
        CanonicalLevel.Debug => 5,
        CanonicalLevel.Info => 9,
        CanonicalLevel.Warn => 13,
        CanonicalLevel.Error => 17,
        CanonicalLevel.Fatal => 21,
        _ => 9,
    };

    /// <summary>
    /// Strict parse: a known level spelling (case-insensitive, surrounding
    /// whitespace trimmed), or null. The alias set is part of the parity
    /// contract — every SDK accepts exactly these spellings.
    /// </summary>
    public static CanonicalLevel? ParseLevel(string level)
    {
        ArgumentNullException.ThrowIfNull(level);

        return level.Trim().ToLowerInvariant() switch
        {
            "trace" or "verbose" => CanonicalLevel.Trace,
            "debug" => CanonicalLevel.Debug,
            "info" or "information" or "log" or "notice" => CanonicalLevel.Info,
            "warn" or "warning" => CanonicalLevel.Warn,
            "error" or "err" => CanonicalLevel.Error,
            "fatal" or "critical" or "crit" or "emergency" or "panic" => CanonicalLevel.Fatal,
            _ => null,
        };
    }

    /// <summary>
    /// Normalize any level spelling to the canonical form. Unknown spellings
    /// become INFO — fail-safe: an unrecognised level must never cause a drop,
    /// and must never be promoted into ERROR (which would corrupt the error
    /// rate).
    /// </summary>
    public static CanonicalLevel NormalizeLevel(string level) => ParseLevel(level) ?? CanonicalLevel.Info;

    /// <summary>True when <paramref name="level"/> is at or above <paramref name="minimum"/>.</summary>
    public static bool MeetsMinimumLevel(CanonicalLevel level, CanonicalLevel minimum) => Rank(level) >= Rank(minimum);

    /// <summary>
    /// The single decision point for "does this log line get emitted?".
    /// </summary>
    /// <remarks>
    /// Order matters and is part of the parity contract:
    /// <list type="number">
    /// <item>kill switch — off means off, no exceptions</item>
    /// <item>minimum level — below the floor is not emitted</item>
    /// <item>WARN/ERROR/FATAL — always 100% (ADR-010: "sampling errors is malpractice")</item>
    /// <item>trace decision, if a trace exists — inherited, never re-rolled</item>
    /// <item>otherwise the session decision — one roll for the whole session</item>
    /// </list>
    /// </remarks>
    public static bool ShouldEmitLog(LogSamplingInput input)
    {
        if (!input.Enabled)
        {
            return false;
        }

        var level = NormalizeLevel(input.Level);
        if (!MeetsMinimumLevel(level, input.MinimumLevel))
        {
            return false;
        }

        if (Rank(level) >= Rank(CanonicalLevel.Warn))
        {
            return true;
        }

        if (input.TraceSampled is { } traceSampled)
        {
            return traceSampled;
        }

        return SampleDecision(input.SessionId, input.LogSamplingRatio);
    }
}
