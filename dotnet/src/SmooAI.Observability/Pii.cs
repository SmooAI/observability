using System.Text.RegularExpressions;

namespace SmooAI.Observability;

/// <summary>
/// PII scrubbing — applied to message strings, breadcrumb messages, and headers
/// before transport. Direct port of the TS <c>pii.ts</c> patterns so both SDKs
/// redact identically. Stays opinionated and minimal; tenants can extend in
/// <c>beforeSend</c>.
/// </summary>
public static partial class Pii
{
    // Bearer tokens.
    [GeneratedRegex(@"Bearer\s+[A-Za-z0-9._-]+", RegexOptions.IgnoreCase)]
    private static partial Regex BearerRegex();

    // password / passwd / pwd = value.
    [GeneratedRegex(@"\b(?:password|passwd|pwd)[""']?\s*[:=]\s*[""']?[^""'&\s]+", RegexOptions.IgnoreCase)]
    private static partial Regex PasswordRegex();

    // token / api-key / apikey / secret = value.
    [GeneratedRegex(@"\b(?:token|api[-_]?key|apikey|secret)[""']?\s*[:=]\s*[""']?[^""'&\s]+", RegexOptions.IgnoreCase)]
    private static partial Regex SecretRegex();

    // OpenAI-style sk- keys.
    [GeneratedRegex(@"\bsk-[A-Za-z0-9]{20,}")]
    private static partial Regex SkKeyRegex();

    private static readonly HashSet<string> SensitiveHeaders = new(StringComparer.OrdinalIgnoreCase)
    {
        "authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
        "x-auth-token",
    };

    /// <summary>
    /// Redact known sensitive substrings from a free-form string.
    /// </summary>
    /// <remarks>
    /// The token/apikey/secret pattern intentionally redacts only the value after
    /// the delimiter (replacing <c>key=value</c> with <c>key=[redacted]</c>),
    /// matching the TS port's effective behavior.
    /// </remarks>
    public static string ScrubString(string? input)
    {
        if (string.IsNullOrEmpty(input))
        {
            return input ?? string.Empty;
        }

        var output = BearerRegex().Replace(input, "Bearer [redacted]");
        output = PasswordRegex().Replace(output, "password=[redacted]");
        output = SecretRegex().Replace(output, RedactAfterDelimiter);
        output = SkKeyRegex().Replace(output, "sk-[redacted]");
        return output;
    }

    /// <summary>
    /// Scrub a header dictionary: sensitive header names are fully redacted, all
    /// other values are passed through <see cref="ScrubString"/>.
    /// </summary>
    public static Dictionary<string, string>? ScrubHeaders(IReadOnlyDictionary<string, string>? headers)
    {
        if (headers is null)
        {
            return null;
        }

        var output = new Dictionary<string, string>(headers.Count, StringComparer.Ordinal);
        foreach (var (key, value) in headers)
        {
            output[key] = SensitiveHeaders.Contains(key) ? "[redacted]" : ScrubString(value);
        }
        return output;
    }

    // Replace everything from the first delimiter onward with =[redacted], keeping
    // the key prefix intact (e.g. "token: abc" -> "token=[redacted]").
    private static string RedactAfterDelimiter(Match match)
    {
        var value = match.Value;
        var delimiterIndex = value.IndexOfAny(new[] { ':', '=' });
        if (delimiterIndex < 0)
        {
            return value;
        }
        return string.Concat(value.AsSpan(0, delimiterIndex), "=[redacted]");
    }
}
