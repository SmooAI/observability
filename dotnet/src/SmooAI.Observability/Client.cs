// SmooAI Observability — .NET SDK.
//
// Port of @smooai/observability's Client (packages/core/src/client.ts). See
// ~/dev/smooai/observability/packages/core/src/ for the canonical TS reference
// and ~/dev/smooai/logger/dotnet/ for SmooAI .NET conventions.

namespace SmooAI.Observability;

/// <summary>
/// Options for <see cref="ObservabilityClient.Init"/>.
/// </summary>
public sealed class ClientOptions
{
    /// <summary>Ingest endpoint: <c>POST /webhooks/observability/{org_id}/{token}</c>.</summary>
    public string Dsn { get; set; } = string.Empty;

    /// <summary>Deployment environment string ('production', 'staging', ...).</summary>
    public string? Environment { get; set; }

    /// <summary>Release id (git sha or deployment version).</summary>
    public string? Release { get; set; }

    /// <summary>Max events kept in memory waiting to flush.</summary>
    public int? MaxQueueSize { get; set; }

    /// <summary>Flush interval in ms (default 1000).</summary>
    public int? FlushIntervalMs { get; set; }

    /// <summary>Max events per flush batch (default 30).</summary>
    public int? MaxBatchSize { get; set; }

    /// <summary>
    /// Drop or mutate events before transport. Return <c>null</c> to drop.
    /// Runs after scope + PII processing.
    /// </summary>
    public Func<ObservabilityEvent, ObservabilityEvent?>? BeforeSend { get; set; }

    /// <summary>
    /// Optional pre-built <see cref="HttpClient"/> for the transport. When null
    /// the client owns and disposes its own.
    /// </summary>
    public HttpClient? HttpClient { get; set; }
}

/// <summary>
/// Extra context for a single capture call.
/// </summary>
public sealed class CaptureContext
{
    /// <summary>One-off tags merged onto the event.</summary>
    public Dictionary<string, string>? Tags { get; set; }
}

/// <summary>
/// A runtime-native capture hook. When registered, captures are routed through
/// this handler IN ADDITION to the HTTP transport — the .NET analogue of the TS
/// Node OTel-native capture path (writes span events). Must never throw.
/// </summary>
/// <param name="ev">The fully-prepared event.</param>
/// <param name="error">The original exception, if this was a CaptureException.</param>
public delegate void CaptureHandler(ObservabilityEvent ev, Exception? error);

/// <summary>
/// Error-capture client. Prepares <see cref="ObservabilityEvent"/>s from
/// exceptions / messages, merges the ambient <see cref="Scope"/>, scrubs PII,
/// and dispatches them to the HTTP transport and/or a registered
/// <see cref="CaptureHandler"/>. Every public method is error-safe — failures
/// are swallowed so observability never crashes the host.
/// </summary>
public sealed class ObservabilityClient
{
    /// <summary>SDK name emitted on every event.</summary>
    public const string SdkName = "@smooai/observability-dotnet";

    /// <summary>SDK version emitted on every event.</summary>
    public const string SdkVersion = "0.1.0";

    private ClientOptions? _options;
    private Transport? _transport;
    private CaptureHandler? _captureHandler;
    private readonly object _gate = new();

    /// <summary>True once <see cref="Init"/> has been called.</summary>
    public bool IsInitialized
    {
        get
        {
            lock (_gate)
            {
                return _options is not null;
            }
        }
    }

    /// <summary>
    /// Initialize the client. Constructs the HTTP transport when a non-empty DSN
    /// is supplied; a blank DSN leaves the transport unwired (OTel-native capture
    /// only). Never throws.
    /// </summary>
    public void Init(ClientOptions options)
    {
        if (options is null)
        {
            return;
        }
        lock (_gate)
        {
            _options = options;
            _transport?.Dispose();
            _transport = null;
            if (!string.IsNullOrEmpty(options.Dsn))
            {
                try
                {
                    var transportOptions = new TransportOptions { Dsn = options.Dsn };
                    if (options.MaxQueueSize is { } q)
                    {
                        transportOptions.MaxQueueSize = q;
                    }
                    if (options.FlushIntervalMs is { } f)
                    {
                        transportOptions.FlushIntervalMs = f;
                    }
                    if (options.MaxBatchSize is { } b)
                    {
                        transportOptions.MaxBatchSize = b;
                    }
                    _transport = new Transport(transportOptions, options.HttpClient);
                }
                catch
                {
                    // Bad DSN etc. — leave transport unwired; capture handler still works.
                    _transport = null;
                }
            }
        }
    }

    /// <summary>Register (or clear with <c>null</c>) the runtime-native capture handler.</summary>
    public void RegisterCaptureHandler(CaptureHandler? handler)
    {
        lock (_gate)
        {
            _captureHandler = handler;
        }
    }

    /// <summary>Set the user on the current scope.</summary>
    public void SetUser(UserContext? user) => ObservabilityContext.SetUser(user);

    /// <summary>Set a tag on the current scope.</summary>
    public void SetTag(string key, string value) => ObservabilityContext.SetTag(key, value);

    /// <summary>Add a breadcrumb to the current scope.</summary>
    public void AddBreadcrumb(string category, string? message = null, Dictionary<string, object?>? data = null, Level level = Level.Info) =>
        ObservabilityContext.AddBreadcrumb(category, message, data, level);

    /// <summary>
    /// Capture an exception. Returns the assigned event id, or <c>null</c> if the
    /// client is uninitialized or the event was dropped by <c>BeforeSend</c>.
    /// Never throws.
    /// </summary>
    public string? CaptureException(Exception error, CaptureContext? context = null)
    {
        try
        {
            ClientOptions? options;
            lock (_gate)
            {
                options = _options;
            }
            if (options is null)
            {
                return null;
            }

            var eventId = Guid.NewGuid().ToString();
            var ev = new ObservabilityEvent
            {
                EventId = eventId,
                Timestamp = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                Level = Level.Error,
                Exception = new List<ExceptionInfo> { ToExceptionInfo(error) },
                Tags = context?.Tags,
                Release = options.Release,
                Environment = options.Environment,
                Sdk = NewSdkInfo(),
            };

            return Dispatch(ev, options, error);
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// Capture a one-line message at the given level. Returns the assigned event
    /// id, or <c>null</c> if uninitialized / dropped. Never throws.
    /// </summary>
    public string? CaptureMessage(string message, Level level = Level.Info)
    {
        try
        {
            ClientOptions? options;
            lock (_gate)
            {
                options = _options;
            }
            if (options is null)
            {
                return null;
            }

            var eventId = Guid.NewGuid().ToString();
            var ev = new ObservabilityEvent
            {
                EventId = eventId,
                Timestamp = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                Level = level,
                Message = Pii.ScrubString(message),
                Release = options.Release,
                Environment = options.Environment,
                Sdk = NewSdkInfo(),
            };

            return Dispatch(ev, options, null);
        }
        catch
        {
            return null;
        }
    }

    /// <summary>Force-flush the transport queue. No-op if no transport is wired.</summary>
    public Task FlushAsync()
    {
        Transport? transport;
        lock (_gate)
        {
            transport = _transport;
        }
        return transport?.FlushAsync() ?? Task.CompletedTask;
    }

    /// <summary>Dispose the transport. Used in shutdown / tests.</summary>
    public void Close()
    {
        lock (_gate)
        {
            _transport?.Dispose();
            _transport = null;
            _options = null;
            _captureHandler = null;
        }
    }

    private string Dispatch(ObservabilityEvent ev, ClientOptions options, Exception? error)
    {
        // Merge ambient scope, then run beforeSend.
        var prepared = ObservabilityContext.GetCurrentScope().ApplyToEvent(ev);
        ScrubBreadcrumbs(prepared);

        var final = options.BeforeSend is not null ? options.BeforeSend(prepared) : prepared;
        if (final is null)
        {
            return ev.EventId;
        }

        // Fire the native handler first so a throwing transport can't suppress it.
        CaptureHandler? handler;
        Transport? transport;
        lock (_gate)
        {
            handler = _captureHandler;
            transport = _transport;
        }

        if (handler is not null)
        {
            try
            {
                handler(final, error);
            }
            catch
            {
                // swallow — observability must not throw
            }
        }

        transport?.Enqueue(final);
        return ev.EventId;
    }

    private static void ScrubBreadcrumbs(ObservabilityEvent ev)
    {
        if (ev.Breadcrumbs is null)
        {
            return;
        }
        foreach (var crumb in ev.Breadcrumbs)
        {
            if (crumb.Message is not null)
            {
                crumb.Message = Pii.ScrubString(crumb.Message);
            }
        }
    }

    private static SdkInfo NewSdkInfo() => new()
    {
        Name = SdkName,
        Version = SdkVersion,
        Runtime = Runtime.Node,
    };

    /// <summary>
    /// Convert an exception (and its <see cref="System.Exception.InnerException"/>
    /// chain) into the <see cref="ExceptionInfo"/> wire shape. Bounded depth so a
    /// cyclic / pathological chain can't loop forever.
    /// </summary>
    internal static ExceptionInfo ToExceptionInfo(Exception error, int depth = 0)
    {
        const int maxDepth = 10;
        var info = new ExceptionInfo
        {
            Type = error.GetType().Name,
            Value = Pii.ScrubString(error.Message),
            Stacktrace = new StackTrace { Frames = StackParser.Parse(error) },
        };
        if (error.InnerException is not null && depth < maxDepth)
        {
            info.Cause = ToExceptionInfo(error.InnerException, depth + 1);
        }
        return info;
    }
}

/// <summary>
/// Process-wide singleton facade, mirroring the TS module-level <c>Client</c>
/// export. Most callers use <c>Sdk.Client</c>; advanced callers can construct
/// their own <see cref="ObservabilityClient"/>.
/// </summary>
public static class Sdk
{
    /// <summary>The shared client instance.</summary>
    public static ObservabilityClient Client { get; } = new();
}
