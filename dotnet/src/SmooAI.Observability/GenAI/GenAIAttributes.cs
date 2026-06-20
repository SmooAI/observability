using System.Diagnostics;

namespace SmooAI.Observability.GenAI;

/// <summary>
/// OTel GenAI semantic-convention attributes. All optional — set what you know.
/// Keys mirror the spec exactly (<c>gen_ai.*</c>) so any GenAI-semconv-aware OTel
/// backend (Datadog, Honeycomb, Phoenix, Smoo's LLM dashboard) parses them
/// without mapping. Port of the TS <c>gen-ai-attributes.ts</c>.
///
/// Spec: https://opentelemetry.io/docs/specs/semconv/gen-ai/
/// </summary>
public sealed class GenAIAttributes
{
    /// <summary>Provider system, e.g. 'openai', 'anthropic'.</summary>
    public string? System { get; set; }

    /// <summary>Operation kind — 'agent', 'chat', 'tool', 'embeddings', etc.</summary>
    public string? OperationName { get; set; }

    /// <summary>Model the request asked for, e.g. 'gpt-4o', 'claude-opus-4-7'.</summary>
    public string? RequestModel { get; set; }

    /// <summary>Model the response actually came from (may differ under fallback).</summary>
    public string? ResponseModel { get; set; }

    /// <summary>Provider-issued response id.</summary>
    public string? ResponseId { get; set; }

    /// <summary>Sampling temperature.</summary>
    public double? Temperature { get; set; }

    /// <summary>Top-P sampling cutoff.</summary>
    public double? TopP { get; set; }

    /// <summary>Top-K sampling cutoff.</summary>
    public double? TopK { get; set; }

    /// <summary>Max generated token budget.</summary>
    public int? MaxTokens { get; set; }

    /// <summary>Random seed, if supplied.</summary>
    public long? Seed { get; set; }

    /// <summary>Input/prompt tokens billed.</summary>
    public int? UsageInputTokens { get; set; }

    /// <summary>Output/completion tokens billed.</summary>
    public int? UsageOutputTokens { get; set; }

    /// <summary>Cached-prompt tokens (prompt-cache hit / cached input).</summary>
    public int? UsageCachedTokens { get; set; }

    /// <summary>Total cost in USD, when available.</summary>
    public double? UsageCostUsd { get; set; }

    /// <summary>Tool names called within this span.</summary>
    public IReadOnlyList<string>? ToolNames { get; set; }

    /// <summary>Truncation flag — true if the response was cut off.</summary>
    public bool? Truncated { get; set; }

    /// <summary>Provider's reported finish reason ('stop', 'length', 'tool_calls', ...).</summary>
    public string? FinishReason { get; set; }

    /// <summary>End-user id from your system (not the provider's).</summary>
    public string? EndUserId { get; set; }

    /// <summary>Conversation/session id.</summary>
    public string? ConversationId { get; set; }
}

/// <summary>
/// Helpers to apply <see cref="GenAIAttributes"/> onto an <see cref="Activity"/>
/// (.NET's span type). Skips unset fields so partial calls are idempotent. Port
/// of the TS <c>setGenAIAttributes</c> / <c>recordGenAIMessage</c>.
/// </summary>
public static class GenAIActivity
{
    /// <summary>Apply GenAI attributes onto the given span. No-op if span is null.</summary>
    public static void SetAttributes(Activity? span, GenAIAttributes attrs)
    {
        if (span is null || attrs is null)
        {
            return;
        }
        SetIf(span, "gen_ai.system", attrs.System);
        SetIf(span, "gen_ai.operation.name", attrs.OperationName);
        SetIf(span, "gen_ai.request.model", attrs.RequestModel);
        SetIf(span, "gen_ai.response.model", attrs.ResponseModel);
        SetIf(span, "gen_ai.response.id", attrs.ResponseId);
        SetIf(span, "gen_ai.request.temperature", attrs.Temperature);
        SetIf(span, "gen_ai.request.top_p", attrs.TopP);
        SetIf(span, "gen_ai.request.top_k", attrs.TopK);
        SetIf(span, "gen_ai.request.max_tokens", attrs.MaxTokens);
        SetIf(span, "gen_ai.request.seed", attrs.Seed);
        SetIf(span, "gen_ai.usage.input_tokens", attrs.UsageInputTokens);
        SetIf(span, "gen_ai.usage.output_tokens", attrs.UsageOutputTokens);
        SetIf(span, "gen_ai.usage.cached_tokens", attrs.UsageCachedTokens);
        SetIf(span, "gen_ai.usage.cost_usd", attrs.UsageCostUsd);
        if (attrs.ToolNames is { Count: > 0 })
        {
            span.SetTag("gen_ai.tool.names", attrs.ToolNames.ToArray());
        }
        SetIf(span, "gen_ai.response.truncated", attrs.Truncated);
        SetIf(span, "gen_ai.response.finish_reason", attrs.FinishReason);
        SetIf(span, "gen_ai.end_user.id", attrs.EndUserId);
        SetIf(span, "gen_ai.conversation.id", attrs.ConversationId);
    }

    /// <summary>
    /// Emit a <c>gen_ai.{role}.message</c> span event carrying the message content
    /// so the dashboard's prompt/completion view can render it. Use sparingly —
    /// these are size-heavy. No-op if span is null.
    /// </summary>
    public static void RecordMessage(Activity? span, string role, string content, string? toolCallId = null, string? toolName = null)
    {
        if (span is null)
        {
            return;
        }
        var tags = new ActivityTagsCollection
        {
            { "gen_ai.message.content", content },
        };
        if (toolCallId is not null)
        {
            tags["gen_ai.tool_call.id"] = toolCallId;
        }
        if (toolName is not null)
        {
            tags["gen_ai.tool.name"] = toolName;
        }
        span.AddEvent(new ActivityEvent($"gen_ai.{role}.message", tags: tags));
    }

    private static void SetIf(Activity span, string key, string? value)
    {
        if (value is not null)
        {
            span.SetTag(key, value);
        }
    }

    private static void SetIf(Activity span, string key, double? value)
    {
        if (value.HasValue)
        {
            span.SetTag(key, value.Value);
        }
    }

    private static void SetIf(Activity span, string key, int? value)
    {
        if (value.HasValue)
        {
            span.SetTag(key, value.Value);
        }
    }

    private static void SetIf(Activity span, string key, long? value)
    {
        if (value.HasValue)
        {
            span.SetTag(key, value.Value);
        }
    }

    private static void SetIf(Activity span, string key, bool? value)
    {
        if (value.HasValue)
        {
            span.SetTag(key, value.Value);
        }
    }
}
