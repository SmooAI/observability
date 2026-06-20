using System.Text.Json.Serialization;

namespace SmooAI.Observability;

/// <summary>
/// Severity level for an <see cref="ObservabilityEvent"/> or <see cref="Breadcrumb"/>.
/// Serialized as the lowercase string the TS SDK emits ('fatal' | 'error' |
/// 'warning' | 'info' | 'debug') so the backend fingerprints events identically
/// regardless of which language SDK produced them.
/// </summary>
[JsonConverter(typeof(LevelConverter))]
public enum Level
{
    /// <summary>fatal</summary>
    Fatal,

    /// <summary>error</summary>
    Error,

    /// <summary>warning</summary>
    Warning,

    /// <summary>info</summary>
    Info,

    /// <summary>debug</summary>
    Debug,
}

/// <summary>
/// The runtime that produced an event. The .NET SDK always reports
/// <see cref="Node"/> — it shares the server-side wire shape with the TS Node
/// runtime (there is no browser .NET runtime in this fleet).
/// </summary>
[JsonConverter(typeof(RuntimeConverter))]
public enum Runtime
{
    /// <summary>browser</summary>
    Browser,

    /// <summary>node</summary>
    Node,
}

/// <summary>
/// Serializes <see cref="Level"/> as the lowercase wire string. Hand-written so
/// the SDK works on net8.0 (the generic <c>JsonStringEnumConverter&lt;T&gt;</c>
/// and <c>JsonStringEnumMemberName</c> are .NET 9+).
/// </summary>
internal sealed class LevelConverter : JsonConverter<Level>
{
    public override Level Read(ref System.Text.Json.Utf8JsonReader reader, Type typeToConvert, System.Text.Json.JsonSerializerOptions options) =>
        reader.GetString() switch
        {
            "fatal" => Level.Fatal,
            "error" => Level.Error,
            "warning" => Level.Warning,
            "info" => Level.Info,
            "debug" => Level.Debug,
            _ => Level.Info,
        };

    public override void Write(System.Text.Json.Utf8JsonWriter writer, Level value, System.Text.Json.JsonSerializerOptions options) =>
        writer.WriteStringValue(value switch
        {
            Level.Fatal => "fatal",
            Level.Error => "error",
            Level.Warning => "warning",
            Level.Info => "info",
            Level.Debug => "debug",
            _ => "info",
        });
}

/// <summary>
/// Serializes <see cref="Runtime"/> as the lowercase wire string. Hand-written
/// for the same net8.0 compatibility reason as <see cref="LevelConverter"/>.
/// </summary>
internal sealed class RuntimeConverter : JsonConverter<Runtime>
{
    public override Runtime Read(ref System.Text.Json.Utf8JsonReader reader, Type typeToConvert, System.Text.Json.JsonSerializerOptions options) =>
        reader.GetString() == "browser" ? Runtime.Browser : Runtime.Node;

    public override void Write(System.Text.Json.Utf8JsonWriter writer, Runtime value, System.Text.Json.JsonSerializerOptions options) =>
        writer.WriteStringValue(value == Runtime.Browser ? "browser" : "node");
}

/// <summary>
/// SDK self-identification, emitted on every event so the dashboard can tell
/// which SDK/version/runtime produced an event.
/// </summary>
public sealed class SdkInfo
{
    /// <summary>SDK package name, e.g. "@smooai/observability-dotnet".</summary>
    public string Name { get; set; } = string.Empty;

    /// <summary>SDK version.</summary>
    public string Version { get; set; } = string.Empty;

    /// <summary>Runtime that produced the event.</summary>
    public Runtime Runtime { get; set; } = Runtime.Node;
}

/// <summary>
/// User / org / session context attached to an event.
/// </summary>
public sealed class UserContext
{
    /// <summary>Application user id.</summary>
    public string? Id { get; set; }

    /// <summary>Organization id (multi-tenant scoping).</summary>
    public string? OrgId { get; set; }

    /// <summary>Session id.</summary>
    public string? SessionId { get; set; }
}

/// <summary>
/// One frame of a parsed stack trace. Innermost (most recent) first to match the
/// <c>@smooai/observability</c> event envelope.
/// </summary>
public sealed class StackFrame
{
    /// <summary>Filename or module identifier (e.g. an assembly or source file).</summary>
    public string Module { get; set; } = string.Empty;

    /// <summary>Function/method name from the stack.</summary>
    public string? Function { get; set; }

    /// <summary>Line number in source, if known.</summary>
    public int? Lineno { get; set; }

    /// <summary>Column number, if known.</summary>
    public int? Colno { get; set; }

    /// <summary>True when the frame is application code (not framework / SDK-internal).</summary>
    public bool? InApp { get; set; }
}

/// <summary>
/// Container for a frame list. Mirrors the TS <c>{ frames: StackFrame[] }</c>
/// shape so the JSON envelope is identical.
/// </summary>
public sealed class StackTrace
{
    /// <summary>Frames, innermost first.</summary>
    public List<StackFrame> Frames { get; set; } = new();
}

/// <summary>
/// A single exception in the chain (innermost first). The <see cref="Cause"/>
/// link walks <see cref="System.Exception.InnerException"/> just as the TS SDK
/// walks <c>Error.cause</c>.
/// </summary>
public sealed class ExceptionInfo
{
    /// <summary>Exception type name (e.g. "InvalidOperationException").</summary>
    public string Type { get; set; } = string.Empty;

    /// <summary>Exception message.</summary>
    public string Value { get; set; } = string.Empty;

    /// <summary>Parsed stack frames.</summary>
    public StackTrace Stacktrace { get; set; } = new();

    /// <summary>Linked cause (inner exception), if any.</summary>
    public ExceptionInfo? Cause { get; set; }
}

/// <summary>
/// A breadcrumb leading up to an event — the recent trail of activity that gives
/// an error context.
/// </summary>
public sealed class Breadcrumb
{
    /// <summary>When the breadcrumb was recorded, ms since epoch.</summary>
    public long Timestamp { get; set; }

    /// <summary>Free-form category — 'fetch', 'navigation', 'console', 'custom', etc.</summary>
    public string Category { get; set; } = string.Empty;

    /// <summary>Severity of the breadcrumb.</summary>
    public Level Level { get; set; } = Level.Info;

    /// <summary>Short human-readable summary.</summary>
    public string? Message { get; set; }

    /// <summary>Free-form structured data.</summary>
    public Dictionary<string, object?>? Data { get; set; }
}

/// <summary>
/// Request context attached to an event (PII-scrubbed by default).
/// </summary>
public sealed class RequestInfo
{
    /// <summary>Full URL or method + path.</summary>
    public string? Url { get; set; }

    /// <summary>HTTP method.</summary>
    public string? Method { get; set; }

    /// <summary>Selected headers (PII-scrubbed).</summary>
    public Dictionary<string, string>? Headers { get; set; }

    /// <summary>Query-string parameters.</summary>
    public string? QueryString { get; set; }
}

/// <summary>
/// The error event envelope. Mirrors the TS <c>ObservabilityEvent</c> field for
/// field so the Smoo ingest endpoint stores .NET and TS events with one schema.
/// </summary>
public sealed class ObservabilityEvent
{
    /// <summary>Client-assigned event id (GUID).</summary>
    public string EventId { get; set; } = string.Empty;

    /// <summary>When the event occurred, ms since epoch.</summary>
    public long Timestamp { get; set; }

    /// <summary>Severity. Most captured exceptions are <see cref="Level.Error"/>.</summary>
    public Level Level { get; set; } = Level.Error;

    /// <summary>One-line message — for <c>CaptureMessage</c>.</summary>
    public string? Message { get; set; }

    /// <summary>Exception chain (innermost first).</summary>
    public List<ExceptionInfo>? Exception { get; set; }

    /// <summary>Breadcrumb buffer leading up to this event.</summary>
    public List<Breadcrumb>? Breadcrumbs { get; set; }

    /// <summary>User context, if known.</summary>
    public UserContext? User { get; set; }

    /// <summary>Request / invocation context.</summary>
    public RequestInfo? Request { get; set; }

    /// <summary>Free-form tags for filtering in the dashboard.</summary>
    public Dictionary<string, string>? Tags { get; set; }

    /// <summary>Free-form contexts (e.g. os, device, runtime).</summary>
    public Dictionary<string, Dictionary<string, object?>>? Contexts { get; set; }

    /// <summary>Release identifier — git sha, deployment version, etc.</summary>
    public string? Release { get; set; }

    /// <summary>Deployment environment.</summary>
    public string? Environment { get; set; }

    /// <summary>SDK self-identification.</summary>
    public SdkInfo Sdk { get; set; } = new();
}

/// <summary>
/// Transport envelope POSTed to the Smoo ingest endpoint. Discriminated union on
/// <c>type</c> with the existing <c>'log'</c> path so one endpoint serves both.
/// </summary>
public sealed class IngestPayload
{
    /// <summary>Discriminator — always "error" for this SDK.</summary>
    public string Type { get; set; } = "error";

    /// <summary>Events in this batch.</summary>
    public List<ObservabilityEvent> Events { get; set; } = new();
}
