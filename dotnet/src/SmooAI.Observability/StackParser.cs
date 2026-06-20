using System.Reflection;
using DiagStackFrame = System.Diagnostics.StackFrame;
using DiagStackTrace = System.Diagnostics.StackTrace;

namespace SmooAI.Observability;

/// <summary>
/// Parses a .NET <see cref="System.Exception"/> into structured
/// <see cref="StackFrame"/>s, innermost-first, to match the
/// <c>@smooai/observability</c> event envelope. Uses
/// <see cref="System.Diagnostics.StackTrace"/> for reliable structured frames
/// (method, declaring type, file, line, column) rather than parsing the
/// <see cref="System.Exception.StackTrace"/> string — the .NET analogue of the
/// TS SDK's engine-specific string parsing.
/// </summary>
public static class StackParser
{
    // Namespace prefixes treated as non-application (framework / runtime).
    // Matched on a namespace-segment boundary (see IsInApp) so a consumer
    // namespace that merely shares a textual prefix is not misclassified.
    private static readonly string[] NonAppPrefixes =
    {
        "System",
        "Microsoft",
        "OpenTelemetry",
    };

    // SDK-internal namespace prefix, dropped from the TOP of the stack so the
    // first frame the dashboard shows is the caller's code, not our capture path.
    private const string SdkInternalPrefix = "SmooAI.Observability";

    /// <summary>
    /// Parse an exception's call stack into frames (innermost first), with
    /// SDK-internal frames dropped from the top.
    /// </summary>
    public static List<StackFrame> Parse(Exception exception)
    {
        ArgumentNullException.ThrowIfNull(exception);

        var frames = new List<StackFrame>();
        DiagStackTrace trace;
        try
        {
            trace = new DiagStackTrace(exception, fNeedFileInfo: true);
        }
        catch
        {
            // No usable trace (e.g. exception never thrown). Fall back to empty.
            return frames;
        }

        foreach (var diagFrame in trace.GetFrames())
        {
            if (diagFrame is null)
            {
                continue;
            }
            frames.Add(ToFrame(diagFrame));
        }

        return DropSdkFrames(frames);
    }

    /// <summary>
    /// Strip SDK-internal frames from the top (innermost) of a stack. Used by
    /// <c>Client.CaptureException</c> so the trace starts at user code.
    /// </summary>
    public static List<StackFrame> DropSdkFrames(List<StackFrame> frames)
    {
        var index = 0;
        while (index < frames.Count
               && frames[index].InApp == false
               && frames[index].Module.StartsWith(SdkInternalPrefix, StringComparison.Ordinal))
        {
            index++;
        }
        return index == 0 ? frames : frames.GetRange(index, frames.Count - index);
    }

    private static StackFrame ToFrame(DiagStackFrame diagFrame)
    {
        MethodBase? method = diagFrame.GetMethod();
        var declaringType = method?.DeclaringType;
        var module = declaringType?.FullName ?? declaringType?.Name ?? "anonymous";
        var function = method?.Name;

        var line = diagFrame.GetFileLineNumber();
        var column = diagFrame.GetFileColumnNumber();
        // Prefer the source file path as the module when available — it's more
        // actionable than the type name for in-app frames.
        var fileName = diagFrame.GetFileName();
        if (!string.IsNullOrEmpty(fileName))
        {
            module = fileName;
        }

        var isInApp = IsInApp(declaringType);

        return new StackFrame
        {
            Module = module,
            Function = function,
            Lineno = line > 0 ? line : null,
            Colno = column > 0 ? column : null,
            InApp = isInApp,
        };
    }

    private static bool IsInApp(Type? declaringType)
    {
        if (declaringType is null)
        {
            return false;
        }
        // The SDK's own code is non-app, identified by assembly (not namespace
        // string) so consumers whose namespace shares the SDK's textual prefix
        // are still treated as app code.
        if (declaringType.Assembly == typeof(StackParser).Assembly)
        {
            return false;
        }
        var fullName = declaringType.FullName;
        if (string.IsNullOrEmpty(fullName))
        {
            return false;
        }
        foreach (var prefix in NonAppPrefixes)
        {
            if (StartsWithNamespaceSegment(fullName!, prefix))
            {
                return false;
            }
        }
        return true;
    }

    // True when fullName equals prefix or begins with "prefix." — i.e. the prefix
    // is a leading namespace segment, not just a textual prefix.
    private static bool StartsWithNamespaceSegment(string fullName, string prefix)
    {
        if (!fullName.StartsWith(prefix, StringComparison.Ordinal))
        {
            return false;
        }
        return fullName.Length == prefix.Length || fullName[prefix.Length] == '.';
    }
}
