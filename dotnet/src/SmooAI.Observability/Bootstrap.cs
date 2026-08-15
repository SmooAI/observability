using SmooAI.Observability.Auth;
using SmooAI.Observability.Otel;

namespace SmooAI.Observability;

/// <summary>
/// Env-var bootstrap config. Mirrors the TS <c>BootstrapEnv</c>. Any field left
/// null falls back to the corresponding <c>SMOOAI_OBSERVABILITY_*</c> env var.
/// </summary>
public sealed class BootstrapEnv
{
    /// <summary>Base ingest URL — SDK appends <c>/v1/traces</c> and <c>/v1/metrics</c>.</summary>
    public string? Endpoint { get; set; }

    /// <summary>Explicit traces endpoint override.</summary>
    public string? TracesEndpoint { get; set; }

    /// <summary>Explicit metrics endpoint override.</summary>
    public string? MetricsEndpoint { get; set; }

    /// <summary>Explicit logs endpoint override.</summary>
    public string? LogsEndpoint { get; set; }

    /// <summary>Pre-minted Bearer JWT (wins over client-credentials when both set).</summary>
    public string? Token { get; set; }

    /// <summary>OAuth <c>/token</c> base URL.</summary>
    public string? AuthUrl { get; set; }

    /// <summary>M2M client id.</summary>
    public string? ClientId { get; set; }

    /// <summary>M2M client secret.</summary>
    public string? ClientSecret { get; set; }

    /// <summary>Service name. Defaults to "smoo-service".</summary>
    public string? ServiceName { get; set; }

    /// <summary>Deployment environment.</summary>
    public string? Environment { get; set; }

    /// <summary>Release id.</summary>
    public string? Release { get; set; }

    /// <summary>Webhook DSN for the error transport (optional).</summary>
    public string? Dsn { get; set; }

    /// <summary>Skip bootstrap entirely.</summary>
    public bool? Disabled { get; set; }

    /// <summary>
    /// HMAC key used to hash emails / phones / addresses in scrubbed strings
    /// (see <see cref="Pii"/>). Unset means personal identifiers are fully
    /// redacted rather than hashed.
    /// </summary>
    public string? PiiHashKey { get; set; }
}

/// <summary>
/// Result of <see cref="Bootstrap.Run"/>. Mirrors the TS <c>BootstrapResult</c>.
/// </summary>
public sealed class BootstrapResult
{
    /// <summary>
    /// Whether the bootstrap actually ran (false = disabled or init threw).
    /// <para>
    /// NOTE: <c>Installed == true</c> only means bootstrap ran. It does NOT mean
    /// anything is being exported — check <see cref="Exporting"/> for that. These
    /// were the same flag until 2026-08-15, and the conflation hid a production
    /// service emitting nothing for months: it had no endpoint configured, so no
    /// OTLP exporter was ever built, yet bootstrap reported success.
    /// </para>
    /// </summary>
    public bool Installed { get; init; }

    /// <summary>
    /// Whether an OTLP exporter was actually installed — i.e. whether spans and
    /// metrics have somewhere to go. False when no endpoint is configured, in
    /// which case this SDK is a no-op for telemetry.
    /// </summary>
    public bool Exporting { get; init; }

    /// <summary>OTel handle (flush/shutdown). Null if init failed or was skipped.</summary>
    public OtelSdkHandle? Otel { get; init; }
}

/// <summary>
/// One-call, env-driven bootstrap for the SDK. Same <c>SMOOAI_OBSERVABILITY_*</c>
/// variables as the TS SDK, so the same deployment config serves both. Idempotent
/// and never throws — missing config / mint / OTel-init failures are written to
/// stderr and the host keeps running.
///
/// Wires: OTel traces+metrics export (with client-credentials auth when
/// configured), the error <see cref="Sdk.Client"/>, and the OTel-native capture
/// handler.
/// </summary>
public static class Bootstrap
{
    private static readonly object Gate = new();
    private static BootstrapResult? _result;

    /// <summary>
    /// Run the bootstrap. Returns the cached result on subsequent calls. Pass
    /// <paramref name="overrides"/> to override env defaults (tests / advanced
    /// callers).
    /// </summary>
    public static async Task<BootstrapResult> Run(BootstrapEnv? overrides = null)
    {
        lock (Gate)
        {
            if (_result is not null)
            {
                return _result;
            }
        }

        var env = ResolveEnv(overrides);

        // Before anything can emit: a scrubbed string written without this key
        // redacts PII outright, so installing it late would silently produce a
        // window of uncorrelatable spans rather than an error.
        Pii.SetPiiHashKey(env.PiiHashKey);

        if (env.Disabled == true)
        {
            return Cache(new BootstrapResult { Installed = false, Exporting = false, Otel = null });
        }

        try
        {
            var (tokenProvider, staticHeaders) = ResolveAuth(env, warn: true);

            // Warm-up mint so the first export doesn't pay the round trip.
            if (tokenProvider is not null)
            {
                try
                {
                    await tokenProvider.GetAccessTokenAsync().ConfigureAwait(false);
                }
                catch (Exception ex)
                {
                    Warn($"initial token mint failed; OTLP exports will retry on first export: {ex.Message}");
                }
            }

            var tracesEndpoint = env.TracesEndpoint ?? (env.Endpoint is not null ? $"{StripSlash(env.Endpoint)}/v1/traces" : null);
            var metricsEndpoint = env.MetricsEndpoint ?? (env.Endpoint is not null ? $"{StripSlash(env.Endpoint)}/v1/metrics" : null);

            // The single most expensive silence this SDK can produce. With no
            // endpoint nothing reaches a collector, yet every other signal (this
            // result, the caller's own "observability enabled" log) still says
            // healthy. Say it plainly and name the variable to set.
            //
            // Only traces + metrics count: ObservabilitySdk.Setup builds an
            // exporter for exactly those two, and a LogsEndpoint alone is
            // consumed by the ILoggingBuilder extension, not here — claiming
            // Exporting on it would be the same lie in a new place.
            var exporting = !string.IsNullOrEmpty(tracesEndpoint) || !string.IsNullOrEmpty(metricsEndpoint);
            if (!exporting)
            {
                Warn(
                    "NO OTLP ENDPOINT CONFIGURED — telemetry is NOT being exported. " +
                    "Nothing this process emits will reach a collector. " +
                    "Set SMOOAI_OBSERVABILITY_ENDPOINT (or a per-signal " +
                    "OTEL_EXPORTER_OTLP_{TRACES,METRICS,LOGS}_ENDPOINT), or set " +
                    "SMOOAI_OBSERVABILITY_DISABLED=true to make this silence deliberate.");
            }

            var otel = ObservabilitySdk.Setup(new SetupOtelOptions
            {
                ServiceName = env.ServiceName ?? "smoo-service",
                Environment = env.Environment,
                Release = env.Release,
                OtlpTracesEndpoint = tracesEndpoint,
                OtlpMetricsEndpoint = metricsEndpoint,
                TokenProvider = tokenProvider,
                OtlpHeaders = staticHeaders,
            });

            Sdk.Client.Init(new ClientOptions
            {
                Dsn = env.Dsn ?? string.Empty,
                Environment = env.Environment ?? "unknown",
                Release = env.Release,
            });

            // Route captures through OTel span events in addition to the webhook.
            OtelCapture.Register();

            // A process that dies from an unhandled exception — or silently drops a
            // faulted Task nobody awaited — reported nothing at all before this.
            // Auto-wired here rather than opt-in for the same reason Rust's
            // bootstrap installs the panic hook: an opt-in crash reporter is one
            // nobody remembers to opt into.
            GlobalHandlers.Register(otel);

            return Cache(new BootstrapResult { Installed = true, Exporting = exporting, Otel = otel });
        }
        catch (Exception ex)
        {
            Warn($"SDK init failed: {ex.Message}");
            return Cache(new BootstrapResult { Installed = false, Exporting = false, Otel = null });
        }
    }

    /// <summary>
    /// Resolve auth from env: a pre-minted Bearer becomes a static header; M2M
    /// client-credentials become a lazy-minting <see cref="TokenProvider"/> (no
    /// warm-up here — the caller decides whether to pre-mint). Shared by
    /// <see cref="Run"/> and the logging extension so all signals authenticate identically.
    /// </summary>
    internal static (TokenProvider? TokenProvider, Dictionary<string, string>? StaticHeaders) ResolveAuth(BootstrapEnv env, bool warn)
    {
        if (!string.IsNullOrEmpty(env.Token))
        {
            return (null, new Dictionary<string, string>(StringComparer.Ordinal)
            {
                ["authorization"] = $"Bearer {env.Token}",
            });
        }

        if (!string.IsNullOrEmpty(env.AuthUrl) && !string.IsNullOrEmpty(env.ClientId) && !string.IsNullOrEmpty(env.ClientSecret))
        {
            return (new TokenProvider(new TokenProviderOptions
            {
                AuthUrl = env.AuthUrl!,
                ClientId = env.ClientId!,
                ClientSecret = env.ClientSecret!,
            }), null);
        }

        if (warn)
        {
            Warn("no auth configured (set SMOOAI_OBSERVABILITY_TOKEN or _AUTH_URL/_CLIENT_ID/_CLIENT_SECRET); OTLP exports will be unauthenticated");
        }
        return (null, null);
    }

    internal static BootstrapEnv ResolveEnv(BootstrapEnv? overrides)
    {
        string? Env(string key) => System.Environment.GetEnvironmentVariable(key);
        return new BootstrapEnv
        {
            Endpoint = overrides?.Endpoint ?? Env("SMOOAI_OBSERVABILITY_ENDPOINT"),
            TracesEndpoint = overrides?.TracesEndpoint ?? Env("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
            MetricsEndpoint = overrides?.MetricsEndpoint ?? Env("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"),
            LogsEndpoint = overrides?.LogsEndpoint ?? Env("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"),
            Token = overrides?.Token ?? Env("SMOOAI_OBSERVABILITY_TOKEN"),
            AuthUrl = overrides?.AuthUrl ?? Env("SMOOAI_OBSERVABILITY_AUTH_URL"),
            ClientId = overrides?.ClientId ?? Env("SMOOAI_OBSERVABILITY_CLIENT_ID"),
            ClientSecret = overrides?.ClientSecret ?? Env("SMOOAI_OBSERVABILITY_CLIENT_SECRET"),
            ServiceName = overrides?.ServiceName ?? Env("SMOOAI_OBSERVABILITY_SERVICE_NAME") ?? "smoo-service",
            Environment = overrides?.Environment ?? Env("SMOOAI_OBSERVABILITY_ENVIRONMENT") ?? Env("STAGE") ?? Env("DOTNET_ENVIRONMENT") ?? Env("ASPNETCORE_ENVIRONMENT"),
            Release = overrides?.Release ?? Env("SMOOAI_OBSERVABILITY_RELEASE") ?? Env("GIT_SHA") ?? Env("LAMBDA_FUNCTION_VERSION") ?? "dev",
            Dsn = overrides?.Dsn ?? Env("SMOOAI_OBSERVABILITY_DSN") ?? Env("OBSERVABILITY_DSN"),
            Disabled = overrides?.Disabled ?? Truthy(Env("SMOOAI_OBSERVABILITY_DISABLED")),
            PiiHashKey = overrides?.PiiHashKey ?? Env("SMOOAI_OBSERVABILITY_PII_HASH_KEY"),
        };
    }

    private static BootstrapResult Cache(BootstrapResult result)
    {
        lock (Gate)
        {
            _result ??= result;
            return _result;
        }
    }

    private static string StripSlash(string url) => url.TrimEnd('/');

    private static bool Truthy(string? value) =>
        !string.IsNullOrEmpty(value) && (value == "1" || value.Equals("true", StringComparison.OrdinalIgnoreCase));

    private static void Warn(string message)
    {
        try
        {
            Console.Error.WriteLine($"[SmooAI.Observability/bootstrap] {message}");
        }
        catch
        {
            // don't crash if even stderr is unavailable
        }
    }

    /// <summary>Test seam — reset bootstrap state.</summary>
    internal static void ResetForTests()
    {
        lock (Gate)
        {
            _result = null;
        }
        // Run() subscribes process-wide crash handlers; leaving them attached would
        // leak across tests (and block the test host's finalizer thread on flush).
        GlobalHandlers.ResetForTests();
    }
}
