using System.Diagnostics;
using SmooAI.Observability.Otel;

namespace SmooAI.Observability;

/// <summary>
/// Process-level unhandled-crash capture. The .NET analogue of the TS
/// <c>registerNodeGlobalHandlers</c> (<c>uncaughtException</c> /
/// <c>unhandledRejection</c>) and the Rust <c>install_panic_hook</c>: without it
/// a service can die from the loudest failure it has and report nothing at all.
///
/// Hooks two events:
/// <list type="bullet">
/// <item><see cref="AppDomain.UnhandledException"/> — an exception that escaped
/// every frame. On .NET Core this is a <em>notification</em>, not an
/// interception: the runtime terminates the process as soon as the handler
/// chain returns and <c>finally</c>/<c>using</c> blocks never run. Nothing can
/// stop the termination, which is exactly why the bounded force-flush below is
/// the whole point of this file.</item>
/// <item><see cref="TaskScheduler.UnobservedTaskException"/> — a faulted
/// <see cref="Task"/> nobody ever awaited. It does not terminate the process on
/// .NET Core, which is precisely why it is the most commonly missed crash class:
/// the work silently did not happen and no one hears about it.</item>
/// </list>
///
/// Both are multicast events, so this <em>subscribes</em>; anything the host
/// already attached still runs and the runtime's own crash output is untouched.
/// A crash reporter that hides the stack trace is worse than no reporter.
/// </summary>
public static class GlobalHandlers
{
    /// <summary>
    /// Default force-flush budget, in ms, spent draining the exporters before the
    /// runtime tears the process down.
    ///
    /// 2000 ms because: (a) .NET Core documents no grace period for
    /// <see cref="AppDomain.UnhandledException"/> — the process dies when the
    /// handler returns, so the budget is simply whatever we take, and everything
    /// we take delays the crash and the supervisor's restart; (b) it is the number
    /// already used everywhere else in this repo (<see cref="OtelSdkHandle.Flush"/>
    /// defaults to 2000, the TS SDK flushes with <c>2_000</c> on SIGTERM/SIGINT),
    /// so one number holds across SDKs; (c) it sits far inside a Kubernetes
    /// <c>terminationGracePeriodSeconds</c> (30s default) while being short enough
    /// that an unreachable collector cannot hold a dying process hostage.
    /// </summary>
    public const int DefaultFlushTimeoutMs = 2000;

    private static readonly object Gate = new();
    private static bool _installed;
    private static OtelSdkHandle? _otel;
    private static int _flushTimeoutMs = DefaultFlushTimeoutMs;

    /// <summary>
    /// Subscribe the crash handlers. Idempotent — a second call is a no-op, since
    /// these are multicast events and subscribing twice would report every crash
    /// twice.
    /// </summary>
    /// <param name="otel">
    /// Handle from <see cref="ObservabilitySdk.Setup"/> / <see cref="Bootstrap.Run"/>,
    /// force-flushed on a crash. Null still reports (the event reaches the
    /// webhook transport), it just cannot drain the OTLP batch processor.
    /// </param>
    /// <param name="flushTimeoutMs">
    /// Flush budget in ms. Values &lt;= 0 fall back to <see cref="DefaultFlushTimeoutMs"/>;
    /// this is a hard bound, never an indefinite block.
    /// </param>
    public static void Register(OtelSdkHandle? otel = null, int flushTimeoutMs = DefaultFlushTimeoutMs)
    {
        lock (Gate)
        {
            if (_installed)
            {
                return;
            }
            _installed = true;
            _otel = otel;
            _flushTimeoutMs = flushTimeoutMs > 0 ? flushTimeoutMs : DefaultFlushTimeoutMs;

            AppDomain.CurrentDomain.UnhandledException += OnUnhandledException;
            TaskScheduler.UnobservedTaskException += OnUnobservedTaskException;
        }
    }

    private static void OnUnhandledException(object? sender, UnhandledExceptionEventArgs e)
    {
        // ExceptionObject is typed `object` because a non-CLS throw is possible;
        // a hard cast here would throw out of a crash handler.
        var error = e.ExceptionObject as Exception
            ?? new InvalidOperationException($"non-Exception unhandled throw: {e.ExceptionObject}");
        Report(error, "AppDomain.UnhandledException", e.IsTerminating);
    }

    private static void OnUnobservedTaskException(object? sender, UnobservedTaskExceptionEventArgs e)
    {
        // Deliberately NOT calling e.SetObserved(): that would change the host's
        // configured behaviour (it is what suppresses a rethrow under
        // <ThrowUnobservedTaskExceptions>). Observability reports, it does not
        // decide whether the process lives.
        Report(Unwrap(e.Exception), "TaskScheduler.UnobservedTaskException", terminating: false);
    }

    /// <summary>
    /// Report the fault, not its wrapper. A single-fault
    /// <see cref="AggregateException"/> would otherwise group every unobserved task
    /// under one type with the message "One or more errors occurred." — losing the
    /// real type, message, and stack the error dashboard groups on.
    /// </summary>
    private static Exception Unwrap(AggregateException aggregate)
    {
        var flat = aggregate.Flatten();
        return flat.InnerExceptions.Count == 1 ? flat.InnerExceptions[0] : flat;
    }

    private static void Report(Exception error, string source, bool terminating)
    {
        try
        {
            // One call covers both halves: the client emits an ERROR-level event to
            // the webhook transport AND fans out to the registered OTel capture
            // handler, which writes the semconv `exception` event onto the current
            // Activity (or a synthetic one) and sets its status to Error.
            Sdk.Client.CaptureException(error, new CaptureContext
            {
                Tags = new Dictionary<string, string>(StringComparer.Ordinal)
                {
                    ["source"] = source,
                    ["terminating"] = terminating ? "true" : "false",
                },
            });

            StopAmbientActivities();
            FlushBounded();
        }
        catch
        {
            // NEVER let anything escape. An exception thrown from an
            // unhandled-exception handler aborts the multicast invocation, so the
            // host's own handler would never run and the reporter would hide the
            // very crash it exists to report.
        }
    }

    /// <summary>
    /// End any in-flight <see cref="Activity"/> so it actually reaches the exporter.
    /// An unhandled exception terminates without unwinding — <c>using</c>/<c>finally</c>
    /// never run — and a batch processor only exports a span on stop, so the span
    /// carrying the exception event would otherwise be flushed into nothing.
    /// </summary>
    private static void StopAmbientActivities()
    {
        // Bounded: Stop() re-points Activity.Current at the parent, but a listener
        // that resurrects Current must not spin us forever inside a crash handler.
        for (var depth = 0; depth < 64; depth++)
        {
            var current = Activity.Current;
            if (current is null)
            {
                return;
            }
            current.Stop();
        }
    }

    private static void FlushBounded()
    {
        var otel = _otel;
        var budget = _flushTimeoutMs;

        // ONE hard deadline for the whole drain, on a pool thread.
        // OtelSdkHandle.Flush applies its timeout per provider (traces AND
        // metrics), so waiting on it inline could cost 2x the budget; and the
        // transport drain would add more on top. The process is terminating, so an
        // abandoned pool thread costs nothing — an unbounded wait would cost the
        // crash itself.
        var drain = Task.Run(() =>
        {
            // Inner catches keep the task from faulting: an unobserved faulted task
            // here would be raised back into OnUnobservedTaskException later.
            try
            {
                otel?.Flush(budget);
            }
            catch
            {
                // best effort
            }
            try
            {
                Sdk.Client.FlushAsync().GetAwaiter().GetResult();
            }
            catch
            {
                // best effort
            }
        });
        drain.Wait(budget);
    }

    /// <summary>Test seam — unsubscribe so the next call re-attaches.</summary>
    internal static void ResetForTests()
    {
        lock (Gate)
        {
            if (!_installed)
            {
                return;
            }
            AppDomain.CurrentDomain.UnhandledException -= OnUnhandledException;
            TaskScheduler.UnobservedTaskException -= OnUnobservedTaskException;
            _installed = false;
            _otel = null;
            _flushTimeoutMs = DefaultFlushTimeoutMs;
        }
    }
}
