using System.Globalization;
using System.Text.Json;
using SmooAI.Observability;

namespace SmooAI.Observability.Tests;

/// <summary>
/// ADR-097 §4 — the .NET lane of the parity corpus.
/// </summary>
/// <remarks>
/// Every SDK (TS, Rust, Python, Go, .NET) asserts against the same
/// <c>parity/sampling-corpus.json</c> in its own CI. A language that cannot
/// reproduce a vector fails its build. Documentation claiming parity is not
/// evidence of parity.
/// </remarks>
public class ParityCorpusTests
{
    // Theory data is the raw JSON text of each vector rather than a JsonElement:
    // strings are xUnit-serializable, so each vector gets its own discovered
    // test case with a readable name, and no JsonDocument has to outlive the
    // MemberData enumeration.
    private static readonly string CorpusJson = File.ReadAllText(FindCorpus());

    private static string FindCorpus()
    {
        // Walk up from the test binary to the repo root — the corpus is shared
        // with four other languages and lives at the top level, not next to the
        // assembly.
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "parity", "sampling-corpus.json");
            if (File.Exists(candidate))
            {
                return candidate;
            }

            dir = dir.Parent;
        }

        throw new FileNotFoundException($"parity/sampling-corpus.json not found above {AppContext.BaseDirectory}");
    }

    private static IEnumerable<object[]> Section(string name)
    {
        using var doc = JsonDocument.Parse(CorpusJson);
        if (!doc.RootElement.TryGetProperty(name, out var section) || section.ValueKind != JsonValueKind.Array)
        {
            throw new InvalidOperationException($"corpus section `{name}` missing or not an array");
        }

        foreach (var vector in section.EnumerateArray())
        {
            yield return new object[] { vector.GetRawText() };
        }
    }

    private static JsonElement Parse(string vectorJson) => JsonDocument.Parse(vectorJson).RootElement.Clone();

    public static IEnumerable<object[]> SampleDecisionVectors() => Section("sampleDecision");

    public static IEnumerable<object[]> NearThresholdVectors() => Section("sampleDecisionNearThreshold");

    public static IEnumerable<object[]> NonFiniteRatioVectors() => Section("sampleDecisionNonFiniteRatio");

    public static IEnumerable<object[]> LevelNormalizationVectors() => Section("levelNormalization");

    public static IEnumerable<object[]> TraceparentParseVectors() => Section("traceparentParse");

    public static IEnumerable<object[]> TraceparentFormatVectors() => Section("traceparentFormat");

    public static IEnumerable<object[]> SettingsResolutionVectors() => Section("settingsResolution");

    public static IEnumerable<object[]> ShouldEmitLogVectors() => Section("shouldEmitLog");

    [Fact]
    public void CorpusIsTheExpectedVersionAndIsNotEmpty()
    {
        using var doc = JsonDocument.Parse(CorpusJson);
        Assert.Equal(1, doc.RootElement.GetProperty("version").GetInt32());
        Assert.True(SampleDecisionVectors().Count() > 50, "the corpus looks truncated");
    }

    [Theory]
    [MemberData(nameof(SampleDecisionVectors))]
    public void SampleDecision(string vectorJson)
    {
        var v = Parse(vectorJson);
        var id = v.GetProperty("id").GetString()!;
        Assert.Equal(v.GetProperty("hash").GetUInt32(), Sampling.Fnv1a32(id));
        Assert.Equal(v.GetProperty("expected").GetBoolean(), Sampling.SampleDecision(id, v.GetProperty("ratio").GetDouble()));
    }

    [Theory]
    [MemberData(nameof(NearThresholdVectors))]
    public void SampleDecisionNearThreshold(string vectorJson)
    {
        var v = Parse(vectorJson);
        var id = v.GetProperty("id").GetString()!;
        var h = Sampling.Fnv1a32(id);
        Assert.Equal(v.GetProperty("hash").GetUInt32(), h);

        // The division is exact in binary64, so this is an equality check, not
        // an epsilon one — drift here means a language got the uint
        // reinterpretation or the divisor wrong.
        Assert.Equal(v.GetProperty("position").GetDouble(), h / 4294967296.0);
        Assert.Equal(v.GetProperty("expected").GetBoolean(), Sampling.SampleDecision(id, v.GetProperty("ratio").GetDouble()));
    }

    [Theory]
    [MemberData(nameof(NonFiniteRatioVectors))]
    public void NonFiniteRatioFailsOpen(string vectorJson)
    {
        var v = Parse(vectorJson);
        var ratio = v.GetProperty("ratio").GetString() switch
        {
            "NaN" => double.NaN,
            "Infinity" => double.PositiveInfinity,
            "-Infinity" => double.NegativeInfinity,
            var other => throw new InvalidOperationException($"corpus names an unknown non-finite ratio: {other}"),
        };
        Assert.Equal(v.GetProperty("expected").GetBoolean(), Sampling.SampleDecision(v.GetProperty("id").GetString()!, ratio));
    }

    [Theory]
    [MemberData(nameof(LevelNormalizationVectors))]
    public void LevelNormalization(string vectorJson)
    {
        var v = Parse(vectorJson);
        Assert.Equal(v.GetProperty("expected").GetString(), Sampling.NormalizeLevel(v.GetProperty("input").GetString()!).ToWire());
    }

    [Theory]
    [MemberData(nameof(TraceparentParseVectors))]
    public void TraceparentParse(string vectorJson)
    {
        var v = Parse(vectorJson);
        var got = Traceparent.Parse(v.GetProperty("input").GetString()!);
        var want = v.GetProperty("expected");

        if (want.ValueKind == JsonValueKind.Null)
        {
            Assert.Null(got);
            return;
        }

        Assert.NotNull(got);
        Assert.Equal(want.GetProperty("traceId").GetString(), got!.Value.TraceId);
        Assert.Equal(want.GetProperty("spanId").GetString(), got.Value.SpanId);
        Assert.Equal(want.GetProperty("flags").GetByte(), got.Value.Flags);
        Assert.Equal(want.GetProperty("sampled").GetBoolean(), got.Value.Sampled);
    }

    [Theory]
    [MemberData(nameof(TraceparentFormatVectors))]
    public void TraceparentFormat(string vectorJson)
    {
        var v = Parse(vectorJson);
        var input = v.GetProperty("input");
        int? flags = input.TryGetProperty("flags", out var f) ? f.GetInt32() : null;
        bool? sampled = input.TryGetProperty("sampled", out var s) ? s.GetBoolean() : null;

        var got = Traceparent.Format(input.GetProperty("traceId").GetString()!, input.GetProperty("spanId").GetString()!, flags, sampled);
        var want = v.GetProperty("expected");
        Assert.Equal(want.ValueKind == JsonValueKind.Null ? null : want.GetString(), got);
    }

    [Theory]
    [MemberData(nameof(SettingsResolutionVectors))]
    public void SettingsResolution(string vectorJson)
    {
        var v = Parse(vectorJson);
        var got = TelemetrySettingsResolver.Resolve(v.GetProperty("input"));
        var want = v.GetProperty("expected");

        Assert.Equal(want.GetProperty("enabled").GetBoolean(), got.Enabled);
        Assert.Equal(want.GetProperty("browserLogSamplingRatio").GetDouble(), got.BrowserLogSamplingRatio);
        Assert.Equal(want.GetProperty("minimumLogLevel").GetString(), got.MinimumLogLevel.ToWire());
        Assert.Equal(want.GetProperty("traceSamplingRatio").GetDouble(), got.TraceSamplingRatio);
    }

    [Theory]
    [MemberData(nameof(ShouldEmitLogVectors))]
    public void ShouldEmitLog(string vectorJson)
    {
        var v = Parse(vectorJson);
        var i = v.GetProperty("input");
        var minimum = Sampling.ParseLevel(i.GetProperty("minimumLevel").GetString()!)
            ?? throw new InvalidOperationException(
                string.Format(CultureInfo.InvariantCulture, "corpus names an unknown canonical level: {0}", i.GetProperty("minimumLevel").GetString()));

        var got = Sampling.ShouldEmitLog(new LogSamplingInput(
            i.GetProperty("level").GetString()!,
            i.GetProperty("sessionId").GetString()!,
            i.GetProperty("enabled").GetBoolean(),
            minimum,
            i.GetProperty("logSamplingRatio").GetDouble(),
            i.TryGetProperty("traceSampled", out var ts) ? ts.GetBoolean() : null));

        Assert.Equal(v.GetProperty("expected").GetBoolean(), got);
    }
}
