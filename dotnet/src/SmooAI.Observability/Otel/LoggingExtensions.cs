using Microsoft.Extensions.Logging;
using OpenTelemetry.Logs;
using SmooAI.Observability.Auth;

namespace SmooAI.Observability.Otel;

/// <summary>
/// Wires the Smoo <b>logs</b> signal into a host's <see cref="ILoggingBuilder"/>.
/// Logs export via OTLP/HTTP to the same ingest endpoint as traces/metrics
/// (<c>SMOOAI_OBSERVABILITY_ENDPOINT</c> → <c>/v1/logs</c>) with the same auth —
/// so a single deployment config drives all three signals. Logs are correlated
/// to traces automatically: the OTel logging provider stamps each record with the
/// active <see cref="System.Diagnostics.Activity"/>'s W3C trace/span id.
///
/// Unlike traces/metrics (standalone providers built in <see cref="ObservabilitySdk.Setup"/>),
/// the logs signal must hook the host's logging pipeline, so it lives here as an
/// <see cref="ILoggingBuilder"/> extension.
/// </summary>
public static class SmooObservabilityLoggingExtensions
{
    /// <summary>
    /// Env-driven wiring. Reads the same <c>SMOOAI_OBSERVABILITY_*</c> variables as
    /// <see cref="Bootstrap"/>; the logs endpoint is <c>OTEL_EXPORTER_OTLP_LOGS_ENDPOINT</c>
    /// or <c>{SMOOAI_OBSERVABILITY_ENDPOINT}/v1/logs</c>. No-op (existing logging
    /// unchanged) when disabled or no endpoint is configured.
    /// </summary>
    public static ILoggingBuilder AddSmooObservability(this ILoggingBuilder builder, BootstrapEnv? overrides = null)
    {
        ArgumentNullException.ThrowIfNull(builder);

        var env = Bootstrap.ResolveEnv(overrides);
        if (env.Disabled == true)
        {
            return builder;
        }

        var logsEndpoint = env.LogsEndpoint
            ?? (env.Endpoint is not null ? $"{env.Endpoint.TrimEnd('/')}/v1/logs" : null);
        if (string.IsNullOrEmpty(logsEndpoint))
        {
            return builder;
        }

        var (tokenProvider, staticHeaders) = Bootstrap.ResolveAuth(env, warn: false);

        return builder.AddSmooObservability(new SetupOtelOptions
        {
            ServiceName = env.ServiceName ?? "smoo-service",
            Environment = env.Environment,
            Release = env.Release,
            OtlpLogsEndpoint = logsEndpoint,
            TokenProvider = tokenProvider,
            OtlpHeaders = staticHeaders,
        });
    }

    /// <summary>
    /// Explicit wiring. Adds the OpenTelemetry logging provider with an OTLP/HTTP
    /// exporter pointed at <see cref="SetupOtelOptions.OtlpLogsEndpoint"/>. No-op when
    /// that endpoint is null/empty, so existing log output is untouched when logs
    /// export isn't configured.
    /// </summary>
    public static ILoggingBuilder AddSmooObservability(this ILoggingBuilder builder, SetupOtelOptions options)
    {
        ArgumentNullException.ThrowIfNull(builder);
        ArgumentNullException.ThrowIfNull(options);

        if (string.IsNullOrEmpty(options.OtlpLogsEndpoint))
        {
            return builder;
        }

        var resourceBuilder = ObservabilitySdk.BuildResource(options);

        builder.AddOpenTelemetry(logging =>
        {
            logging.SetResourceBuilder(resourceBuilder);
            // Turn structured logging state + scopes into log-record attributes
            // (→ product `parsed_fields`), and keep the rendered message as the body.
            logging.IncludeScopes = true;
            logging.ParseStateValues = true;
            logging.IncludeFormattedMessage = true;
            logging.AddOtlpExporter(exporter =>
                ObservabilitySdk.ConfigureExporter(exporter, options.OtlpLogsEndpoint!, options));
        });

        return builder;
    }
}
