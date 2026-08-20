namespace SmooAI.Observability;

/// <summary>A parsed W3C <c>traceparent</c>.</summary>
/// <param name="TraceId">32 lowercase hex chars.</param>
/// <param name="SpanId">16 lowercase hex chars.</param>
/// <param name="Flags">trace-flags byte, 0-255.</param>
/// <param name="Sampled">bit 0 of <paramref name="Flags"/> — the upstream sampling decision, which logs inherit.</param>
public readonly record struct TraceContext(string TraceId, string SpanId, byte Flags, bool Sampled);

/// <summary>
/// W3C <c>traceparent</c> parse / format. .NET port of
/// <c>packages/core/src/traceparent.ts</c>, pinned by
/// <c>parity/sampling-corpus.json</c>.
/// </summary>
/// <remarks>
/// <para>
/// <c>00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01</c> — version
/// (2 hex), trace-id (32 lowercase hex), span-id (16 lowercase hex), trace-flags
/// (2 hex).
/// </para>
/// <para>
/// Parsing is STRICT — exactly four dash-separated fields, version exactly
/// <c>00</c>. Rejected: wrong field count, wrong version (including the
/// forbidden <c>ff</c>), non-hex or wrong-length fields, uppercase hex, an
/// all-zero trace id, an all-zero span id. The all-zero ids are the classic
/// "propagated a placeholder" bug: accepting them produces traces that all
/// collide on <c>000…0</c>.
/// </para>
/// </remarks>
public static class Traceparent
{
    private const string Version = "00";

    // Convert.ToByte and int.Parse both accept uppercase hex, so the case check
    // has to be explicit.
    private static bool IsLowerHex(string s, int length)
    {
        if (s.Length != length)
        {
            return false;
        }

        foreach (var c in s)
        {
            if (c is not (>= '0' and <= '9') and not (>= 'a' and <= 'f'))
            {
                return false;
            }
        }

        return true;
    }

    private static bool IsAllZero(string s)
    {
        foreach (var c in s)
        {
            if (c != '0')
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>
    /// Parse a <c>traceparent</c> header. Returns null for anything invalid.
    /// </summary>
    public static TraceContext? Parse(string header)
    {
        ArgumentNullException.ThrowIfNull(header);

        var parts = header.Split('-');
        if (parts.Length != 4)
        {
            return null;
        }

        var (version, traceId, spanId, flagsHex) = (parts[0], parts[1], parts[2], parts[3]);
        if (version != Version)
        {
            return null;
        }

        if (!IsLowerHex(traceId, 32) || IsAllZero(traceId))
        {
            return null;
        }

        if (!IsLowerHex(spanId, 16) || IsAllZero(spanId))
        {
            return null;
        }

        if (!IsLowerHex(flagsHex, 2))
        {
            return null;
        }

        var flags = Convert.ToByte(flagsHex, 16);
        return new TraceContext(traceId, spanId, flags, (flags & 0x01) == 0x01);
    }

    /// <summary>
    /// Format a <c>traceparent</c> header.
    /// </summary>
    /// <remarks>
    /// <paramref name="flags"/> is an <c>int?</c> rather than a <c>byte?</c> so
    /// an out-of-byte-range value from a caller is rejected rather than silently
    /// truncated. Pass null to derive the flags byte from
    /// <paramref name="sampled"/>.
    /// <para>
    /// Returns null rather than emitting a header a spec-compliant peer would
    /// reject — a malformed traceparent breaks correlation downstream just as
    /// thoroughly as a missing one, but silently.
    /// </para>
    /// </remarks>
    public static string? Format(string traceId, string spanId, int? flags = null, bool? sampled = null)
    {
        var resolved = flags ?? (sampled == true ? 1 : 0);
        if (resolved is < 0 or > 255)
        {
            return null;
        }

        var header = $"{Version}-{traceId}-{spanId}-{resolved:x2}";

        // Round-trip through the parser so format can never emit what parse rejects.
        return Parse(header) is null ? null : header;
    }
}
