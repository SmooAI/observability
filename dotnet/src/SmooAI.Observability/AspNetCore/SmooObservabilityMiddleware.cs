using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;

namespace SmooAI.Observability.AspNetCore;

/// <summary>
/// ASP.NET Core middleware that, per request: forks a <see cref="Scope"/>, attaches
/// request context (method, path, query, scrubbed headers) as a breadcrumb, and
/// captures any unhandled exception via <see cref="Sdk.Client"/> before rethrowing
/// so the host's own error handling still runs. The .NET framework integration
/// analogue of the TS browser/node auto-instrumentation.
/// </summary>
public sealed class SmooObservabilityMiddleware
{
    private readonly RequestDelegate _next;
    private readonly ObservabilityClient _client;

    /// <summary>Create the middleware. Defaults to the shared <c>Sdk.Client</c>.</summary>
    public SmooObservabilityMiddleware(RequestDelegate next, ObservabilityClient? client = null)
    {
        _next = next ?? throw new ArgumentNullException(nameof(next));
        _client = client ?? Sdk.Client;
    }

    /// <summary>Middleware entry point.</summary>
    public async Task InvokeAsync(HttpContext context)
    {
        ArgumentNullException.ThrowIfNull(context);

        await ObservabilityContext.WithScopeAsync(async scope =>
        {
            try
            {
                AttachRequestContext(scope, context);
            }
            catch
            {
                // Context extraction must never break the request.
            }

            try
            {
                await _next(context).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                // Capture, then rethrow so the host's error handling still runs.
                _client.CaptureException(ex, new CaptureContext
                {
                    Tags = new Dictionary<string, string>(StringComparer.Ordinal)
                    {
                        ["http.method"] = context.Request.Method,
                        ["http.route"] = context.Request.Path.Value ?? "/",
                    },
                });
                throw;
            }
        }).ConfigureAwait(false);
    }

    private static void AttachRequestContext(Scope scope, HttpContext context)
    {
        var request = context.Request;
        var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var header in request.Headers)
        {
            headers[header.Key] = header.Value.ToString();
        }
        var scrubbed = Pii.ScrubHeaders(headers);

        var data = new Dictionary<string, object?>(StringComparer.Ordinal)
        {
            ["method"] = request.Method,
            ["path"] = request.Path.Value,
            ["queryString"] = request.QueryString.HasValue ? request.QueryString.Value : null,
        };
        if (scrubbed is not null)
        {
            data["headers"] = scrubbed;
        }

        scope.AddBreadcrumb(new Breadcrumb
        {
            Category = "http",
            Level = Level.Info,
            Message = $"{request.Method} {request.Path.Value}",
            Data = data,
            Timestamp = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        });
    }
}

/// <summary>
/// Extension to register <see cref="SmooObservabilityMiddleware"/>.
/// </summary>
public static class SmooObservabilityMiddlewareExtensions
{
    /// <summary>
    /// Add the SmooAI observability middleware to the pipeline. Place it early so
    /// it wraps downstream middleware and captures their exceptions.
    /// </summary>
    public static IApplicationBuilder UseSmooObservability(this IApplicationBuilder app)
    {
        ArgumentNullException.ThrowIfNull(app);
        return app.UseMiddleware<SmooObservabilityMiddleware>();
    }
}
