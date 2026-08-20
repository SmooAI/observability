using System.Text;
using System.Text.Json;
using SmooAI.Observability;

namespace SmooAI.Observability.Tests;

public class PiiTests
{
    [Fact]
    public void ScrubString_RedactsBearerTokens()
    {
        var result = Pii.ScrubString("auth header: Bearer abc.def-123_XYZ");
        Assert.Contains("Bearer [redacted]", result);
        Assert.DoesNotContain("abc.def-123_XYZ", result);
    }

    [Fact]
    public void ScrubString_RedactsPassword()
    {
        var result = Pii.ScrubString("login password=hunter2 done");
        Assert.Contains("password=[redacted]", result);
        Assert.DoesNotContain("hunter2", result);
    }

    [Theory]
    [InlineData("token=secretvalue")]
    [InlineData("api_key: abc123")]
    [InlineData("apikey=xyz")]
    [InlineData("secret = topsecret")]
    public void ScrubString_RedactsSecretValues(string input)
    {
        var result = Pii.ScrubString(input);
        Assert.Contains("[redacted]", result);
    }

    [Fact]
    public void ScrubString_RedactsSkKeys()
    {
        var result = Pii.ScrubString("key sk-ABCDEFGHIJKLMNOPQRSTUVWX rest");
        Assert.Contains("sk-[redacted]", result);
        Assert.DoesNotContain("ABCDEFGHIJKLMNOPQRSTUVWX", result);
    }

    [Fact]
    public void ScrubString_LeavesCleanStringUntouched()
    {
        const string clean = "nothing sensitive here";
        Assert.Equal(clean, Pii.ScrubString(clean));
    }

    [Fact]
    public void ScrubString_HandlesNullAndEmpty()
    {
        Assert.Equal(string.Empty, Pii.ScrubString(null));
        Assert.Equal(string.Empty, Pii.ScrubString(string.Empty));
    }

    [Fact]
    public void ScrubHeaders_FullyRedactsSensitiveHeaders()
    {
        var headers = new Dictionary<string, string>
        {
            ["Authorization"] = "Bearer xyz",
            ["Cookie"] = "session=abc",
            ["X-Api-Key"] = "key",
            ["Content-Type"] = "application/json",
        };

        var scrubbed = Pii.ScrubHeaders(headers);

        Assert.NotNull(scrubbed);
        Assert.Equal("[redacted]", scrubbed!["Authorization"]);
        Assert.Equal("[redacted]", scrubbed["Cookie"]);
        Assert.Equal("[redacted]", scrubbed["X-Api-Key"]);
        Assert.Equal("application/json", scrubbed["Content-Type"]);
    }

    [Fact]
    public void ScrubHeaders_IsCaseInsensitiveOnHeaderName()
    {
        var headers = new Dictionary<string, string> { ["AUTHORIZATION"] = "Bearer xyz" };
        var scrubbed = Pii.ScrubHeaders(headers);
        Assert.Equal("[redacted]", scrubbed!["AUTHORIZATION"]);
    }

    [Fact]
    public void ScrubHeaders_ReturnsNullForNull()
    {
        Assert.Null(Pii.ScrubHeaders(null));
    }
}

/// <summary>
/// PII hashing parity with <c>rust/observability/src/pii.rs</c>.
/// </summary>
/// <remarks>
/// These drive the internal <c>ScrubWithKey</c> / <c>TokenWithKey</c> rather than
/// installing the process-wide key: <c>SetPiiHashKey</c> is set-once and xUnit
/// runs the assembly in one process, so a global write would make the suite
/// order-dependent.
/// </remarks>
public class PiiHashingTests
{
    private static readonly byte[] Key = "test-hmac-key-not-a-real-secret"u8.ToArray();
    private static readonly byte[] OtherKey = "a-different-test-hmac-key"u8.ToArray();

    private static string Scrub(string input, string org = "org-1") => Pii.ScrubWithKey(input, org, Key);

    [Fact]
    public void CredentialsAreDroppedNotHashed()
    {
        // A live token must never become a correlatable handle — hashing one still
        // yields an oracle you can test candidate tokens against.
        var result = Scrub("Bearer abc.def-ghi_123 password=hunter2 sk-ABCDEFGHIJKLMNOPQRSTUVWX");
        Assert.Contains("Bearer [redacted]", result);
        Assert.Contains("password=[redacted]", result);
        Assert.Contains("sk-[redacted]", result);
        Assert.DoesNotContain("[token:", result);
        Assert.DoesNotContain("[credential:", result);
        Assert.DoesNotContain("[email:", result);
        Assert.DoesNotContain("[phone:", result);

        // And a personal identifier hiding inside a secret goes with it.
        var inSecret = Scrub("token=a@b.com");
        Assert.DoesNotContain("a@b.com", inSecret);
        Assert.DoesNotContain("[email:", inSecret);
    }

    [Fact]
    public void HashesEmailsKeepingTheTypePrefix()
    {
        var result = Scrub("contact me at Alice@Example.com please");
        Assert.DoesNotContain("alice@example.com", result.ToLowerInvariant());
        Assert.StartsWith("contact me at [email:", result);
        Assert.EndsWith("] please", result);
    }

    [Theory]
    [InlineData("555-0142")]
    [InlineData("(415) 555-0142")]
    [InlineData("+1 415-555-0142")]
    public void HashesPhoneNumbers(string raw)
    {
        var result = Scrub($"call {raw} today");
        Assert.Contains("[phone:", result);
        Assert.DoesNotContain("0142", result);
    }

    [Fact]
    public void HashesStreetAddresses()
    {
        var result = Scrub("ship to 1600 Pennsylvania Ave, Washington");
        Assert.Contains("[address:", result);
        Assert.DoesNotContain("Pennsylvania", result);
    }

    [Fact]
    public void SameValueSameOrgIsStable()
    {
        Assert.Equal(Scrub("a@b.com"), Scrub("a@b.com"));
        // …and correlation survives formatting differences in phones.
        Assert.Equal(Scrub("(415) 555-0142"), Scrub("415-555-0142"));
    }

    [Fact]
    public void SameValueDifferentOrgHashesDifferently()
    {
        var a = Scrub("a@b.com", "org-1");
        var b = Scrub("a@b.com", "org-2");
        Assert.NotEqual(a, b);
        Assert.StartsWith("[email:", a);
        Assert.StartsWith("[email:", b);
    }

    [Fact]
    public void DifferentKeyHashesDifferently()
    {
        Assert.NotEqual(
            Pii.ScrubWithKey("a@b.com", "org-1", Key),
            Pii.ScrubWithKey("a@b.com", "org-1", OtherKey));
    }

    [Fact]
    public void NoKeyRedactsRatherThanHashing()
    {
        // Fail safe: an unkeyed digest of an email is rainbow-tabled instantly.
        var result = Pii.ScrubWithKey("a@b.com and 555-0142", "org-1", null);
        Assert.Contains("[email:redacted]", result);
        Assert.Contains("[phone:redacted]", result);
        Assert.DoesNotContain("a@b.com", result);
        Assert.DoesNotContain("0142", result);
    }

    [Fact]
    public void HashIsShortHex()
    {
        var token = Pii.TokenWithKey(PiiKind.Email, "a@b.com", "org-1", Key);
        var hex = token["[email:".Length..^1];
        Assert.Equal(Pii.HashHexLen, hex.Length);
        Assert.All(hex, c => Assert.Contains(c, "0123456789abcdef"));
    }

    [Fact]
    public void PiiTokenMatchesWhatScrubbingWrote()
    {
        // The searchability contract: hashing a typed query term must produce
        // exactly the token stored in the span.
        var scrubbed = Pii.ScrubWithKey("mail a@b.com now", "org-7", Key);
        var term = Pii.TokenWithKey(PiiKind.Email, "A@B.com ", "org-7", Key);
        Assert.Contains(term, scrubbed);
    }

    [Theory]
    [InlineData("took 1234 ms")]
    [InlineData("version 1.2.3")]
    [InlineData("2026-08-15T12:00:00Z")]
    [InlineData("total 1234.5678")]
    [InlineData("trace 0af7651916cd43dd8448eb211c80319c")]
    public void OrdinaryNumbersAreNotMistakenForPhones(string value)
    {
        // The phone pattern requires a separator before the last 4 digits; ids,
        // versions, dates and amounts must survive intact.
        Assert.Equal(value, Scrub(value));
    }

    [Fact]
    public void SetPiiHashKeyIsSetOnceAndRejectsEmpty()
    {
        Assert.False(Pii.SetPiiHashKey((byte[]?)null));
        Assert.False(Pii.SetPiiHashKey(Array.Empty<byte>()));
        Assert.False(Pii.SetPiiHashKey((string?)null));

        Pii.ResetPiiHashKeyForTests();
        try
        {
            Assert.True(Pii.SetPiiHashKey("first-key"));
            Assert.False(Pii.SetPiiHashKey("second-key"));
            // The first key is what is still in force.
            Assert.Equal(
                Pii.TokenWithKey(PiiKind.Email, "a@b.com", "o", "first-key"u8.ToArray()),
                Pii.PiiToken(PiiKind.Email, "a@b.com", "o"));
        }
        finally
        {
            Pii.ResetPiiHashKeyForTests();
        }
    }

    [Fact]
    public void ScrubHeadersForOrgNeverLeaksRawEmail()
    {
        var headers = new Dictionary<string, string> { ["X-Note"] = "reply to a@b.com" };
        var scrubbed = Pii.ScrubHeadersForOrg(headers, "org-1")!;
        Assert.DoesNotContain("a@b.com", scrubbed["X-Note"]);
        Assert.Contains("[email:", scrubbed["X-Note"]);
    }
}

/// <summary>
/// The .NET lane of the shared PII corpus (ADR-097 §4).
/// </summary>
/// <remarks>
/// The vectors used to be seven tuples typed out in five languages. Nothing
/// detected a divergence in the <em>set</em> — only in values someone remembered
/// to copy. They now live in <c>parity/pii-corpus.json</c>, which all five SDKs
/// load.
/// </remarks>
public class PiiCorpusTests
{
    private static readonly string CorpusJson = File.ReadAllText(FindCorpus());

    private static string FindCorpus()
    {
        // Walk up from the test binary to the repo root — the corpus is shared
        // with four other languages and lives at the top level.
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "parity", "pii-corpus.json");
            if (File.Exists(candidate))
            {
                return candidate;
            }

            dir = dir.Parent;
        }

        throw new FileNotFoundException($"parity/pii-corpus.json not found above {AppContext.BaseDirectory}");
    }

    private static byte[] Key => Encoding.UTF8.GetBytes(Root.GetProperty("hash").GetProperty("key").GetString()!);

    private static byte[] OtherKey => Encoding.UTF8.GetBytes(Root.GetProperty("hash").GetProperty("otherKey").GetString()!);

    private static JsonElement Root => JsonDocument.Parse(CorpusJson).RootElement.Clone();

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

    public static IEnumerable<object[]> TokenWithKeyVectors() => Section("tokenWithKey");

    public static IEnumerable<object[]> TokenWithOtherKeyVectors() => Section("tokenWithOtherKey");

    public static IEnumerable<object[]> TokenWithoutKeyVectors() => Section("tokenWithoutKey");

    private static PiiKind KindFromLabel(string label) => label switch
    {
        "email" => PiiKind.Email,
        "phone" => PiiKind.Phone,
        "address" => PiiKind.Address,
        _ => throw new InvalidOperationException($"corpus names an unknown PII kind: {label}"),
    };

    [Fact]
    public void CorpusIsTheExpectedVersionAndIsNotEmpty()
    {
        Assert.Equal(1, Root.GetProperty("version").GetInt32());
        Assert.NotEmpty(TokenWithKeyVectors());
    }

    [Theory]
    [MemberData(nameof(TokenWithKeyVectors))]
    public void TokenWithKey(string vectorJson)
    {
        var v = JsonDocument.Parse(vectorJson).RootElement;
        var got = Pii.TokenWithKey(KindFromLabel(v.GetProperty("kind").GetString()!), v.GetProperty("raw").GetString()!, v.GetProperty("orgId").GetString()!, Key);
        Assert.Equal(v.GetProperty("expected").GetString(), got);
    }

    [Theory]
    [MemberData(nameof(TokenWithOtherKeyVectors))]
    public void TokenWithOtherKey(string vectorJson)
    {
        var v = JsonDocument.Parse(vectorJson).RootElement;
        var got = Pii.TokenWithKey(
            KindFromLabel(v.GetProperty("kind").GetString()!),
            v.GetProperty("raw").GetString()!,
            v.GetProperty("orgId").GetString()!,
            OtherKey);
        Assert.Equal(v.GetProperty("expected").GetString(), got);
    }

    /// <summary>
    /// No key installed → redaction, never a hash under a guessable key.
    /// </summary>
    [Theory]
    [MemberData(nameof(TokenWithoutKeyVectors))]
    public void TokenWithoutKey(string vectorJson)
    {
        var v = JsonDocument.Parse(vectorJson).RootElement;
        var got = Pii.TokenWithKey(KindFromLabel(v.GetProperty("kind").GetString()!), v.GetProperty("raw").GetString()!, v.GetProperty("orgId").GetString()!, null);
        Assert.Equal(v.GetProperty("expected").GetString(), got);
    }
}
