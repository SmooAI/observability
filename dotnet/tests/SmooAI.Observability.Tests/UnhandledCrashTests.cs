using System.Diagnostics;
using System.Net;
using System.Text;

namespace SmooAI.Observability.Tests;

/// <summary>
/// Records raw OTLP/HTTP request bodies on a loopback port.
///
/// Bodies are kept as BYTES, not text: the payload is protobuf, and the point of
/// these tests is to assert on what actually left the crashing process. Protobuf
/// string fields are length-prefixed UTF-8, so substring matching over a Latin1
/// (byte-for-byte) decode finds them without taking a protobuf dependency.
/// </summary>
internal sealed class OtlpRecorder : IDisposable
{
    private readonly HttpListener _listener;
    private readonly TimeSpan _responseDelay;
    private readonly List<byte[]> _bodies = new();
    private readonly object _gate = new();
    private readonly CancellationTokenSource _cts = new();

    /// <param name="responseDelay">
    /// How long to sit on each request before answering. Non-zero simulates a
    /// wedged collector — the case the bounded flush exists for.
    /// </param>
    public OtlpRecorder(TimeSpan? responseDelay = null)
    {
        _responseDelay = responseDelay ?? TimeSpan.Zero;
        var port = FreePort();
        BaseUrl = $"http://127.0.0.1:{port}";
        _listener = new HttpListener();
        _listener.Prefixes.Add($"http://127.0.0.1:{port}/");
        _listener.Start();
        _ = Task.Run(AcceptLoopAsync);
    }

    /// <summary>Base URL — the SDK appends <c>/v1/traces</c> and <c>/v1/metrics</c>.</summary>
    public string BaseUrl { get; }

    public IReadOnlyList<byte[]> Bodies
    {
        get
        {
            lock (_gate)
            {
                return _bodies.ToArray();
            }
        }
    }

    /// <summary>Every body concatenated, decoded byte-for-byte for substring search.</summary>
    public string Text
    {
        get
        {
            lock (_gate)
            {
                return string.Concat(_bodies.Select(b => Encoding.Latin1.GetString(b)));
            }
        }
    }

    private async Task AcceptLoopAsync()
    {
        while (!_cts.IsCancellationRequested)
        {
            HttpListenerContext context;
            try
            {
                context = await _listener.GetContextAsync().ConfigureAwait(false);
            }
            catch
            {
                return; // listener stopped
            }

            // Per-request task so a deliberately wedged response cannot stall the
            // accept loop for the other signal's exporter.
            _ = Task.Run(() => HandleAsync(context));
        }
    }

    private async Task HandleAsync(HttpListenerContext context)
    {
        try
        {
            using var buffer = new MemoryStream();
            await context.Request.InputStream.CopyToAsync(buffer).ConfigureAwait(false);
            var body = buffer.ToArray();
            lock (_gate)
            {
                _bodies.Add(body);
            }

            if (_responseDelay > TimeSpan.Zero)
            {
                await Task.Delay(_responseDelay, _cts.Token).ConfigureAwait(false);
            }

            context.Response.StatusCode = 200;
            context.Response.ContentType = "application/x-protobuf";
            context.Response.ContentLength64 = 0;
            context.Response.Close();
        }
        catch
        {
            try
            {
                context.Response.Abort();
            }
            catch
            {
                // ignore
            }
        }
    }

    private static int FreePort()
    {
        var listener = new System.Net.Sockets.TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var port = ((IPEndPoint)listener.LocalEndpoint).Port;
        listener.Stop();
        return port;
    }

    public void Dispose()
    {
        _cts.Cancel();
        try
        {
            _listener.Stop();
            _listener.Close();
        }
        catch
        {
            // ignore
        }
        _cts.Dispose();
    }
}

/// <summary>
/// Unhandled-crash reporting, verified the only way it can honestly be verified:
/// by launching a child process that really dies and asserting on what reached
/// the collector, plus the child's exit code and stderr.
/// </summary>
public class UnhandledCrashTests
{
    private static readonly TimeSpan ChildTimeout = TimeSpan.FromSeconds(30);

    private sealed record ChildRun(int ExitCode, string StdOut, string StdErr, bool Killed, TimeSpan Elapsed);

    private static ChildRun RunChild(string scenario, string endpoint, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? extraEnv = null)
    {
        var budget = timeout ?? ChildTimeout;
        var dll = typeof(UnhandledCrashTests).Assembly.Location;
        var dir = Path.GetDirectoryName(dll)!;
        var apphost = Path.Combine(dir, Path.GetFileNameWithoutExtension(dll) + (OperatingSystem.IsWindows() ? ".exe" : string.Empty));

        var psi = new ProcessStartInfo
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            WorkingDirectory = dir,
        };
        if (File.Exists(apphost))
        {
            psi.FileName = apphost;
        }
        else
        {
            psi.FileName = "dotnet";
            psi.ArgumentList.Add("exec");
            psi.ArgumentList.Add(dll);
        }
        psi.ArgumentList.Add(CrashChild.Flag);
        psi.ArgumentList.Add(scenario);
        psi.ArgumentList.Add(endpoint);
        // Don't let an ambient opt-out on the dev box silently make these vacuous.
        psi.Environment["SMOOAI_OBSERVABILITY_DISABLED"] = "0";
        if (extraEnv is not null)
        {
            foreach (var (key, value) in extraEnv)
            {
                psi.Environment[key] = value;
            }
        }

        var clock = Stopwatch.StartNew();
        using var proc = Process.Start(psi)!;
        var stdout = proc.StandardOutput.ReadToEndAsync();
        var stderr = proc.StandardError.ReadToEndAsync();

        var exited = proc.WaitForExit((int)budget.TotalMilliseconds);
        var killed = false;
        if (!exited)
        {
            killed = true;
            try
            {
                proc.Kill(entireProcessTree: true);
            }
            catch
            {
                // already gone
            }
            proc.WaitForExit(5_000);
        }
        clock.Stop();

        Task.WaitAll([stdout, stderr], TimeSpan.FromSeconds(10));
        return new ChildRun(
            killed ? -1 : proc.ExitCode,
            stdout.IsCompletedSuccessfully ? stdout.Result : string.Empty,
            stderr.IsCompletedSuccessfully ? stderr.Result : string.Empty,
            killed,
            clock.Elapsed);
    }

    /// <summary>Poll until the collector has what we expect (the POST lands slightly before the child dies).</summary>
    private static bool WaitFor(Func<bool> condition, TimeSpan timeout)
    {
        var clock = Stopwatch.StartNew();
        while (clock.Elapsed < timeout)
        {
            if (condition())
            {
                return true;
            }
            Thread.Sleep(50);
        }
        return condition();
    }

    /// <summary>
    /// Proves the span STATUS is Error, which a substring search cannot.
    /// OTLP <c>Span.status</c> is a <c>Status</c> submessage carrying
    /// <c>message</c> (field 2, LEN) then <c>code</c> (field 3, varint), so an
    /// Error status described by <paramref name="description"/> is on the wire as
    /// exactly <c>0x12 &lt;len&gt; &lt;utf8&gt; 0x18 0x02</c>.
    /// </summary>
    private static bool HasErrorStatus(IReadOnlyList<byte[]> bodies, string description)
    {
        var text = Encoding.UTF8.GetBytes(description);
        Assert.True(text.Length < 128, "marker must fit a single-byte protobuf length prefix");

        var needle = new byte[text.Length + 4];
        needle[0] = 0x12;
        needle[1] = (byte)text.Length;
        text.CopyTo(needle, 2);
        needle[^2] = 0x18;
        needle[^1] = 0x02; // STATUS_CODE_ERROR

        return bodies.Any(body => body.AsSpan().IndexOf(needle.AsSpan()) >= 0);
    }

    private static int CountOccurrences(string haystack, string needle)
    {
        var count = 0;
        for (var i = haystack.IndexOf(needle, StringComparison.Ordinal); i >= 0; i = haystack.IndexOf(needle, i + needle.Length, StringComparison.Ordinal))
        {
            count++;
        }
        return count;
    }

    [Fact]
    public void MainThreadUnhandledException_ExportsSemconvExceptionWithErrorStatus_BeforeTermination()
    {
        using var collector = new OtlpRecorder();

        var run = RunChild("main-throw", collector.BaseUrl);

        Assert.False(run.Killed, $"child hung; stderr: {run.StdErr}");
        // .NET Core cannot prevent termination from UnhandledException — it is a
        // notification. The process must still die, loudly.
        Assert.NotEqual(0, run.ExitCode);
        Assert.Contains(CrashChild.MainThrowMarker, run.StdErr, StringComparison.Ordinal);
        Assert.Contains("Unhandled exception", run.StdErr, StringComparison.OrdinalIgnoreCase);

        Assert.True(
            WaitFor(() => collector.Text.Contains(CrashChild.MainThrowMarker, StringComparison.Ordinal), TimeSpan.FromSeconds(10)),
            $"nothing carrying the crash reached the collector before the process died; stderr: {run.StdErr}");

        var text = collector.Text;
        Assert.Contains("exception.type", text, StringComparison.Ordinal);
        Assert.Contains("exception.message", text, StringComparison.Ordinal);
        Assert.Contains("AppDomain.UnhandledException", text, StringComparison.Ordinal);
        Assert.True(HasErrorStatus(collector.Bodies, CrashChild.MainThrowMarker), "the exported span status must be Error");
    }

    [Fact]
    public void CrashInsideAmbientActivity_LandsOnThatActivity_AndTheActivityStillExports()
    {
        using var collector = new OtlpRecorder();

        var run = RunChild("activity-throw", collector.BaseUrl);

        Assert.False(run.Killed, $"child hung; stderr: {run.StdErr}");
        Assert.NotEqual(0, run.ExitCode);
        Assert.DoesNotContain("NO-ACTIVITY-LISTENER", run.StdErr, StringComparison.Ordinal);

        Assert.True(
            WaitFor(() => collector.Text.Contains(CrashChild.ActivityThrowMarker, StringComparison.Ordinal), TimeSpan.FromSeconds(10)),
            $"the in-flight activity never reached the collector; stderr: {run.StdErr}");

        var text = collector.Text;
        // The exception rode the ambient span, not a synthetic one.
        Assert.Contains(CrashChild.ActivityName, text, StringComparison.Ordinal);
        Assert.DoesNotContain("observability.captureException", text, StringComparison.Ordinal);
        Assert.True(HasErrorStatus(collector.Bodies, CrashChild.ActivityThrowMarker), "the ambient span must be marked Error");
    }

    [Fact]
    public void UnobservedTaskException_IsReported_AndTheProcessBehaviourIsUnchanged()
    {
        using var collector = new OtlpRecorder();

        var run = RunChild("unobserved-task", collector.BaseUrl);

        Assert.False(run.Killed, $"child hung; stderr: {run.StdErr}");
        // We report, we do not decide: SetObserved is never called, and an
        // unobserved task exception does not terminate on .NET Core.
        Assert.Equal(0, run.ExitCode);

        Assert.True(
            WaitFor(() => collector.Text.Contains(CrashChild.TaskMarker, StringComparison.Ordinal), TimeSpan.FromSeconds(10)),
            $"the unobserved task fault was never reported; stderr: {run.StdErr}");
        Assert.Contains("TaskScheduler.UnobservedTaskException", collector.Text, StringComparison.Ordinal);
        Assert.True(HasErrorStatus(collector.Bodies, CrashChild.TaskMarker), "the exported span status must be Error");
    }

    [Fact]
    public void AlreadyRegisteredHostHandler_StillRuns()
    {
        using var collector = new OtlpRecorder();

        var run = RunChild("host-handler", collector.BaseUrl);

        Assert.False(run.Killed, $"child hung; stderr: {run.StdErr}");
        Assert.Contains(CrashChild.HostHandlerMarker, run.StdErr, StringComparison.Ordinal);
        Assert.Contains(CrashChild.MainThrowMarker, run.StdErr, StringComparison.Ordinal);
        Assert.True(
            WaitFor(() => collector.Text.Contains(CrashChild.MainThrowMarker, StringComparison.Ordinal), TimeSpan.FromSeconds(10)),
            "the SDK handler must run alongside the host's, not instead of it");
    }

    [Fact]
    public void DoubleInstall_ReportsTheCrashExactlyOnce()
    {
        using var collector = new OtlpRecorder();

        var run = RunChild("double-install", collector.BaseUrl);

        Assert.False(run.Killed, $"child hung; stderr: {run.StdErr}");
        Assert.True(
            WaitFor(() => collector.Text.Contains(CrashChild.MainThrowMarker, StringComparison.Ordinal), TimeSpan.FromSeconds(10)),
            $"the crash was never reported at all; stderr: {run.StdErr}");

        // One `smoo.event_id` attribute per recorded capture — two subscriptions
        // to a multicast event would report the same crash twice.
        Assert.Equal(1, CountOccurrences(collector.Text, "smoo.event_id"));
    }

    [Fact]
    public void WedgedCollector_StaysBounded_AndNeitherMasksTheCrashNorLaterHandlers()
    {
        // Accepts the POST and then sits on it far longer than the flush budget.
        using var collector = new OtlpRecorder(TimeSpan.FromSeconds(45));

        // Push the OTLP exporter's OWN request timeout way out (default 10s).
        // Without this the exporter's timeout is what ends the flush, so the test
        // passes even with our budget removed — it would prove nothing about the
        // bound this test is named for.
        var run = RunChild(
            "wedged-exporter",
            collector.BaseUrl,
            extraEnv: new Dictionary<string, string> { ["OTEL_EXPORTER_OTLP_TIMEOUT"] = "60000" });

        Assert.False(run.Killed, "the flush must be bounded — a wedged collector cannot hold a dying process hostage");
        Assert.True(run.Elapsed < TimeSpan.FromSeconds(20), $"child took {run.Elapsed.TotalSeconds:F1}s; the flush budget is not bounded");

        // A reporter that fails must not eat the runtime's crash output, and must
        // not abort the multicast chain before the host's own handler runs.
        Assert.Contains(CrashChild.MainThrowMarker, run.StdErr, StringComparison.Ordinal);
        Assert.Contains(CrashChild.HostHandlerMarker, run.StdErr, StringComparison.Ordinal);
        Assert.NotEqual(0, run.ExitCode);
    }
}
