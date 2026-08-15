using System.Diagnostics;
using System.Runtime.CompilerServices;
using SmooAI.Observability.Otel;

namespace SmooAI.Observability.Tests;

/// <summary>
/// The child-process side of <see cref="UnhandledCrashTests"/>.
///
/// A crash reporter cannot be proven from inside the process that is supposed to
/// be dying: invoking the handler method directly proves neither that it is wired
/// to the runtime nor that the export completes before termination. So the tests
/// re-launch THIS assembly as a subprocess (see <see cref="Flag"/>), let it really
/// crash, and assert on what reached the collector plus the child's exit code and
/// stderr.
/// </summary>
internal static class CrashChild
{
    internal const string Flag = "--crash-child";

    /// <summary>Thrown on the main thread. Unique so it can be found in a protobuf body.</summary>
    internal const string MainThrowMarker = "smoo-child-main-thread-boom";

    /// <summary>Thrown on the main thread while an Activity is current.</summary>
    internal const string ActivityThrowMarker = "smoo-child-in-activity-boom";

    /// <summary>Faulted on a Task nobody ever awaits.</summary>
    internal const string TaskMarker = "smoo-child-unobserved-task-boom";

    /// <summary>Printed by a handler the HOST registered, to prove ours did not displace it.</summary>
    internal const string HostHandlerMarker = "HOST-HANDLER-RAN";

    /// <summary>Name of the ambient activity in the <c>activity-throw</c> scenario.</summary>
    internal const string ActivityName = "child.work";

    internal static int Run(string[] args)
    {
        if (args.Length < 3 || args[0] != Flag)
        {
            // Normal `dotnet test` load — VSTest never calls this entry point anyway.
            return 0;
        }

        var scenario = args[1];
        var endpoint = args[2];

        if (scenario == "host-handler")
        {
            // Registered BEFORE the SDK: the host is first in the multicast chain.
            AppDomain.CurrentDomain.UnhandledException += (_, _) => Console.Error.WriteLine(HostHandlerMarker);
        }

        // Blocking, not awaiting: everything below must stay on the main thread's
        // synchronous stack so the crash is a genuine main-thread unhandled
        // exception with the ambient Activity still current.
        var result = Bootstrap.Run(new BootstrapEnv
        {
            Endpoint = endpoint,
            Token = "child-test-token",
            ServiceName = "crash-child",
            Environment = "test",
            Release = "test",
            Disabled = false,
        }).GetAwaiter().GetResult();

        if (!result.Installed)
        {
            Console.Error.WriteLine("BOOTSTRAP-FAILED");
            return 3;
        }

        switch (scenario)
        {
            case "double-install":
                // Second install must be a no-op; if it subscribes again the crash
                // is reported twice.
                GlobalHandlers.Register(result.Otel);
                break;

            case "wedged-exporter":
                // Registered AFTER the SDK, so OUR handler runs first. This is the
                // ordering where a throw out of the reporter would abort the
                // multicast chain and silently eat the host's crash output.
                AppDomain.CurrentDomain.UnhandledException += (_, _) => Console.Error.WriteLine(HostHandlerMarker);
                break;

            default:
                break;
        }

        switch (scenario)
        {
            case "main-throw":
            case "host-handler":
            case "double-install":
            case "wedged-exporter":
                throw new InvalidOperationException(MainThrowMarker);

            case "activity-throw":
            {
                var source = new ActivitySource(OtelSdkHandle.ActivitySourceName);
                // Deliberately NOT in a `using`: an unhandled exception terminates
                // without unwinding, so a `using` here would be a lie about what
                // happens at crash time — and the whole point is that the SDK has
                // to stop this activity itself or it never reaches the exporter.
                var activity = source.StartActivity(ActivityName, ActivityKind.Internal);
                if (activity is null)
                {
                    Console.Error.WriteLine("NO-ACTIVITY-LISTENER");
                    return 4;
                }
                throw new InvalidOperationException(ActivityThrowMarker);
            }

            case "unobserved-task":
                CreateFaultedTaskAndDropIt();
                // Force the finalizer to raise TaskScheduler.UnobservedTaskException.
                GC.Collect();
                GC.WaitForPendingFinalizers();
                GC.Collect();
                GC.WaitForPendingFinalizers();
                return 0;

            default:
                Console.Error.WriteLine($"UNKNOWN-SCENARIO:{scenario}");
                return 2;
        }
    }

    // Separate frame on purpose: in a Debug build the JIT keeps locals alive to the
    // end of their method, so the task can only become collectable once this
    // returns. Inlining would defeat that.
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void CreateFaultedTaskAndDropIt()
    {
        var task = Task.Run((Action)(() => throw new InvalidOperationException(TaskMarker)));
        var clock = Stopwatch.StartNew();
        while (!task.IsCompleted && clock.ElapsedMilliseconds < 5_000)
        {
            // IsCompleted does not observe the exception — awaiting it would.
            Thread.Sleep(10);
        }
    }
}
