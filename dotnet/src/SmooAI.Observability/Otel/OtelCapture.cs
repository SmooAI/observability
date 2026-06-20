using System.Diagnostics;

namespace SmooAI.Observability.Otel;

/// <summary>
/// OTel-native capture handler. Registered as the client's
/// <see cref="CaptureHandler"/> so every captured exception / message also lands
/// on the active <see cref="Activity"/> (or a synthetic one) as a span
/// exception/event with error status and OTLP-shaped attributes. Port of the TS
/// node <c>registerOtelCapture</c>.
///
/// This fires IN ADDITION to the HTTP transport (when both are wired) so the OTel
/// pipeline gets the structured span for tracing while the webhook gets the event
/// for the Errors dashboard.
/// </summary>
public static class OtelCapture
{
    private static readonly ActivitySource Source = new(OtelSdkHandle.ActivitySourceName);
    private static readonly object Gate = new();
    private static bool _installed;

    /// <summary>
    /// Install the OTel capture handler on the given client (defaults to the
    /// shared <c>Sdk.Client</c>). Idempotent.
    /// </summary>
    public static void Register(ObservabilityClient? client = null)
    {
        lock (Gate)
        {
            if (_installed)
            {
                return;
            }
            _installed = true;
            (client ?? Sdk.Client).RegisterCaptureHandler(Handle);
        }
    }

    private static void Handle(ObservabilityEvent ev, Exception? error)
    {
        var active = Activity.Current;
        if (active is not null)
        {
            RecordOnSpan(active, ev, error);
            return;
        }

        // No active span — mint a synthetic one so the error still surfaces in a
        // trace (background workers capturing outside a request hit this path).
        var name = (ev.Exception?.Count ?? 0) > 0 ? "observability.captureException" : "observability.captureMessage";
        using var span = Source.StartActivity(name, ActivityKind.Internal);
        if (span is not null)
        {
            RecordOnSpan(span, ev, error);
        }
    }

    private static void RecordOnSpan(Activity span, ObservabilityEvent ev, Exception? error)
    {
        var isException = (ev.Exception?.Count ?? 0) > 0;

        span.SetTag("smoo.event_id", ev.EventId);
        if (!string.IsNullOrEmpty(ev.Environment))
        {
            span.SetTag("deployment.environment.name", ev.Environment);
        }
        if (!string.IsNullOrEmpty(ev.Release))
        {
            span.SetTag("service.version", ev.Release);
        }
        span.SetTag("smoo.level", LevelString(ev.Level));
        if (ev.User?.Id is not null)
        {
            span.SetTag("enduser.id", ev.User.Id);
        }
        if (ev.User?.OrgId is not null)
        {
            span.SetTag("enduser.org_id", ev.User.OrgId);
        }
        if (ev.User?.SessionId is not null)
        {
            span.SetTag("enduser.session_id", ev.User.SessionId);
        }
        if (ev.Tags is not null)
        {
            foreach (var (k, v) in ev.Tags)
            {
                span.SetTag($"smoo.tag.{k}", v);
            }
        }

        if (isException)
        {
            if (error is not null)
            {
                RecordException(span, error);
            }
            span.SetStatus(ActivityStatusCode.Error, error?.Message ?? ev.Exception?[0].Value);
        }
        else if (ev.Message is not null)
        {
            span.AddEvent(new ActivityEvent("smoo.message", tags: new ActivityTagsCollection
            {
                { "smoo.message", ev.Message },
            }));
            if (ev.Level is Level.Error or Level.Fatal)
            {
                span.SetStatus(ActivityStatusCode.Error, ev.Message);
            }
        }
    }

    // Portable exception recording (Activity.AddException is .NET 9+). Emits the
    // OTel-standard `exception` span event with type/message/stacktrace tags so
    // any OTLP backend renders it consistently across TFMs.
    private static void RecordException(Activity span, Exception error)
    {
        span.AddEvent(new ActivityEvent("exception", tags: new ActivityTagsCollection
        {
            { "exception.type", error.GetType().FullName ?? error.GetType().Name },
            { "exception.message", error.Message },
            { "exception.stacktrace", error.ToString() },
        }));
    }

    private static string LevelString(Level level) => level switch
    {
        Level.Fatal => "fatal",
        Level.Error => "error",
        Level.Warning => "warning",
        Level.Info => "info",
        Level.Debug => "debug",
        _ => "info",
    };

    /// <summary>Test seam — un-register so the next call re-installs cleanly.</summary>
    internal static void ResetForTests()
    {
        lock (Gate)
        {
            _installed = false;
            Sdk.Client.RegisterCaptureHandler(null);
        }
    }
}
