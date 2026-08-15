using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;

namespace SmooAI.Observability;

/// <summary>
/// The class of personal identifier a match represents. Drives both the visible
/// prefix in the output token and the normalization applied before hashing, so
/// <c>(415) 555-0142</c> and <c>415-555-0142</c> correlate.
/// </summary>
public enum PiiKind
{
    /// <summary>Email address.</summary>
    Email,

    /// <summary>Telephone number.</summary>
    Phone,

    /// <summary>Street address.</summary>
    Address,
}

/// <summary>
/// PII scrubbing — applied to message strings, breadcrumb messages, and headers
/// before transport. Port of <c>rust/observability/src/pii.rs</c>; the semantics
/// are identical across the five SDKs.
/// </summary>
/// <remarks>
/// <para>Two classes, handled differently on purpose:</para>
/// <list type="bullet">
/// <item><description><b>Credentials</b> (<c>Bearer …</c>, <c>password=</c>,
/// <c>token</c>/<c>api_key</c>/<c>secret=</c>, <c>sk-…</c>) are <b>dropped</b>.
/// A hash of a live token is still a token oracle, and there is no correlation
/// value in a secret.</description></item>
/// <item><description><b>Personal identifiers</b> (email, phone, street address)
/// are <b>hashed</b>, not dropped: <c>a@b.com</c> → <c>[email:9f2a41c8]</c>. That
/// keeps the one question worth asking — "are these two spans the same person?" —
/// answerable while storing nothing reversible.</description></item>
/// </list>
/// <para>
/// The hash is <b>HMAC-SHA256, keyed</b>, not a bare digest: emails and phone
/// numbers are a small enumerable space that a rainbow table reverses in seconds.
/// The org id is mixed into the HMAC message, so identical PII hashes
/// <b>differently in different orgs</b>.
/// </para>
/// <para>
/// <b>The key and the org salt are load-bearing and must not rotate casually.</b>
/// Rotating either silently breaks correlation with every previously stored hash.
/// Supply the key once at startup via <c>SMOOAI_OBSERVABILITY_PII_HASH_KEY</c>
/// (read by <see cref="Bootstrap"/>) or <see cref="SetPiiHashKey(byte[])"/>.
/// <b>With no key configured, personal identifiers are fully redacted</b>
/// (<c>[email:redacted]</c>) rather than hashed under a guessable one — fail safe,
/// never fail open.
/// </para>
/// </remarks>
public static partial class Pii
{
    // ---- credentials: matched FIRST, dropped entirely ----------------------
    //
    // A personal identifier sitting inside a secret (token=a@b.com) is dropped
    // with the secret rather than surviving as a hash.

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

    // ---- personal identifiers: hashed, not dropped -------------------------

    [GeneratedRegex(@"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+\b", RegexOptions.IgnoreCase)]
    private static partial Regex EmailRegex();

    // Phone: optional country code / area code, then the 3-4 local pair. A
    // separator is REQUIRED so bare digit runs (ids, timestamps, amounts) don't
    // get eaten.
    [GeneratedRegex(@"(?:\+\d{1,3}[ .-]?)?(?:\(\d{3}\)[ .-]?|\b\d{3}[ .-])?\b\d{3}[ .-]\d{4}\b")]
    private static partial Regex PhoneRegex();

    // US-style street address: house number, 1-3 words, a street suffix.
    [GeneratedRegex(
        @"\b\d{1,6}\s+(?:[A-Za-z0-9.'-]+\s+){0,3}(?:street|st|avenue|ave|road|rd|boulevard|blvd|lane|ln|drive|dr|court|ct|way|terrace|ter|place|pl|circle|cir|highway|hwy|parkway|pkwy|square|sq)\b\.?",
        RegexOptions.IgnoreCase)]
    private static partial Regex AddressRegex();

    [GeneratedRegex(@"[^0-9]")]
    private static partial Regex NonDigitRegex();

    [GeneratedRegex(@"\s+")]
    private static partial Regex WhitespaceRunRegex();

    private static readonly HashSet<string> SensitiveHeaders = new(StringComparer.OrdinalIgnoreCase)
    {
        "authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
        "x-auth-token",
    };

    /// <summary>
    /// Hex characters kept from the HMAC. Long enough that collisions are rare
    /// across an org's traces, short enough to read in a span attribute.
    /// </summary>
    internal const int HashHexLen = 8;

    private static byte[]? _piiHashKey;

    /// <summary>
    /// Install the process-wide HMAC key used to hash personal identifiers.
    /// Idempotent and set-once: returns <c>false</c> if a key was already
    /// installed (the existing key is kept — a mid-process rotation would
    /// silently fork every correlation) or if <paramref name="key"/> is empty.
    /// <see cref="Bootstrap"/> calls this from
    /// <c>SMOOAI_OBSERVABILITY_PII_HASH_KEY</c>.
    /// </summary>
    public static bool SetPiiHashKey(byte[]? key)
    {
        if (key is null || key.Length == 0)
        {
            return false;
        }
        return Interlocked.CompareExchange(ref _piiHashKey, (byte[])key.Clone(), null) is null;
    }

    /// <summary>
    /// <see cref="SetPiiHashKey(byte[])"/> from a UTF-8 string.
    /// </summary>
    public static bool SetPiiHashKey(string? key) =>
        !string.IsNullOrEmpty(key) && SetPiiHashKey(Encoding.UTF8.GetBytes(key));

    /// <summary>
    /// Hash one known-personal value into its scrubbed token — the same token
    /// <see cref="ScrubStringForOrg"/> would have written. This is how a UI
    /// search box finds stored hashes: hash the typed term with the same org and
    /// match. Returns <c>[&lt;kind&gt;:redacted]</c> when no key is installed.
    /// </summary>
    public static string PiiToken(PiiKind kind, string raw, string orgId) =>
        TokenWithKey(kind, raw, orgId, Volatile.Read(ref _piiHashKey));

    /// <summary>
    /// Scrub a free-form string with no org context — credentials dropped,
    /// personal identifiers hashed under the empty org salt. Prefer
    /// <see cref="ScrubStringForOrg"/> wherever an org id is in hand, so hashes
    /// can't be correlated across tenants.
    /// </summary>
    public static string ScrubString(string? input) => ScrubWithKey(input, string.Empty, Volatile.Read(ref _piiHashKey));

    /// <summary>
    /// Scrub a free-form string, salting personal-identifier hashes with
    /// <paramref name="orgId"/>.
    /// </summary>
    public static string ScrubStringForOrg(string? input, string orgId) =>
        ScrubWithKey(input, orgId, Volatile.Read(ref _piiHashKey));

    /// <summary>
    /// Scrub a header dictionary: sensitive header names are fully redacted, all
    /// other values are passed through <see cref="ScrubString"/>.
    /// </summary>
    public static Dictionary<string, string>? ScrubHeaders(IReadOnlyDictionary<string, string>? headers) =>
        ScrubHeadersForOrg(headers, string.Empty);

    /// <summary>
    /// <see cref="ScrubHeaders"/> with an org salt for the personal-identifier
    /// hashes.
    /// </summary>
    public static Dictionary<string, string>? ScrubHeadersForOrg(IReadOnlyDictionary<string, string>? headers, string orgId)
    {
        if (headers is null)
        {
            return null;
        }

        var output = new Dictionary<string, string>(headers.Count, StringComparer.Ordinal);
        foreach (var (key, value) in headers)
        {
            output[key] = SensitiveHeaders.Contains(key) ? "[redacted]" : ScrubStringForOrg(value, orgId);
        }
        return output;
    }

    // Test seam: drives the scrub with an explicit key so the suite doesn't have
    // to write the set-once process-wide key (xUnit runs the class in one
    // process, which would make a global write order-dependent).
    internal static string ScrubWithKey(string? input, string orgId, byte[]? key)
    {
        if (string.IsNullOrEmpty(input))
        {
            return input ?? string.Empty;
        }

        // Credentials first — dropped, never hashed.
        var output = BearerRegex().Replace(input, "Bearer [redacted]");
        output = PasswordRegex().Replace(output, "password=[redacted]");
        output = SecretRegex().Replace(output, RedactAfterDelimiter);
        output = SkKeyRegex().Replace(output, "sk-[redacted]");

        // Personal identifiers — hashed, prefix preserved.
        output = EmailRegex().Replace(output, m => TokenWithKey(PiiKind.Email, m.Value, orgId, key));
        output = PhoneRegex().Replace(output, m => TokenWithKey(PiiKind.Phone, m.Value, orgId, key));
        output = AddressRegex().Replace(output, m => TokenWithKey(PiiKind.Address, m.Value, orgId, key));
        return output;
    }

    internal static string TokenWithKey(PiiKind kind, string raw, string orgId, byte[]? key)
    {
        var label = Label(kind);
        if (key is null || key.Length == 0)
        {
            return $"[{label}:redacted]";
        }

        // orgId in the HMAC message IS the per-org salt. The kind is in there too
        // so a phone and an address that normalize alike can't collide.
        var message = new List<byte>();
        message.AddRange(Encoding.UTF8.GetBytes(orgId));
        message.Add(0);
        message.AddRange(Encoding.UTF8.GetBytes(label));
        message.Add(0);
        message.AddRange(Encoding.UTF8.GetBytes(Normalize(kind, raw)));

        var mac = HMACSHA256.HashData(key, message.ToArray());
        var hex = Convert.ToHexString(mac, 0, HashHexLen / 2).ToLowerInvariant();
        return $"[{label}:{hex}]";
    }

    // Test seam: clears the set-once key so the set-once behavior itself can be
    // asserted. Not part of the public surface — rotation is never valid at runtime.
    internal static void ResetPiiHashKeyForTests() => Volatile.Write(ref _piiHashKey, null);

    /// <summary>The prefix that stays visible in the scrubbed output.</summary>
    internal static string Label(PiiKind kind) => kind switch
    {
        PiiKind.Email => "email",
        PiiKind.Phone => "phone",
        _ => "address",
    };

    private static string Normalize(PiiKind kind, string raw) => kind switch
    {
        // Digits only: formatting must not fork the hash.
        PiiKind.Phone => NonDigitRegex().Replace(raw, string.Empty),
        PiiKind.Email => raw.Trim().ToLowerInvariant(),
        // Case-fold and collapse runs of whitespace.
        _ => WhitespaceRunRegex().Replace(raw.Trim(), " ").ToLowerInvariant(),
    };

    // Replace everything from the first delimiter onward with =[redacted], keeping
    // the key prefix intact (e.g. "token: abc" -> "token=[redacted]").
    private static string RedactAfterDelimiter(Match match)
    {
        var value = match.Value;
        var delimiterIndex = value.IndexOfAny([':', '=']);
        if (delimiterIndex < 0)
        {
            return value;
        }
        return string.Concat(value.AsSpan(0, delimiterIndex), "=[redacted]");
    }
}
