using System.Diagnostics;
using System.Linq;
using SmooAI.Observability.GenAI;

namespace SmooAI.Observability.Tests;

public class GenAITests
{
    // A listener is required for Activity.StartActivity to return a non-null span.
    private static ActivitySource StartListening(out ActivityListener listener)
    {
        var source = new ActivitySource($"test-{Guid.NewGuid()}");
        listener = new ActivityListener
        {
            ShouldListenTo = s => s == source,
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllData,
        };
        ActivitySource.AddActivityListener(listener);
        return source;
    }

    [Fact]
    public void SetAttributes_AppliesGenAiTags()
    {
        var source = StartListening(out var listener);
        using (listener)
        using (var span = source.StartActivity("llm.call"))
        {
            Assert.NotNull(span);
            GenAIActivity.SetAttributes(span, new GenAIAttributes
            {
                System = "anthropic",
                OperationName = "chat",
                RequestModel = "claude-opus-4-8",
                UsageInputTokens = 100,
                UsageOutputTokens = 50,
                Temperature = 0.7,
                ToolNames = new[] { "search", "calc" },
            });

            Assert.Equal("anthropic", span!.GetTagItem("gen_ai.system"));
            Assert.Equal("chat", span.GetTagItem("gen_ai.operation.name"));
            Assert.Equal("claude-opus-4-8", span.GetTagItem("gen_ai.request.model"));
            Assert.Equal(100, span.GetTagItem("gen_ai.usage.input_tokens"));
            Assert.Equal(50, span.GetTagItem("gen_ai.usage.output_tokens"));
            Assert.Equal(0.7, span.GetTagItem("gen_ai.request.temperature"));
        }
    }

    [Fact]
    public void SetAttributes_SkipsUnsetFields()
    {
        var source = StartListening(out var listener);
        using (listener)
        using (var span = source.StartActivity("llm.call"))
        {
            GenAIActivity.SetAttributes(span, new GenAIAttributes { System = "openai" });
            Assert.Null(span!.GetTagItem("gen_ai.request.model"));
            Assert.Null(span.GetTagItem("gen_ai.usage.input_tokens"));
        }
    }

    [Fact]
    public void SetAttributes_NullSpan_IsNoOp()
    {
        // Must not throw.
        GenAIActivity.SetAttributes(null, new GenAIAttributes { System = "x" });
    }

    [Fact]
    public void RecordMessage_AddsSpanEvent()
    {
        var source = StartListening(out var listener);
        using (listener)
        using (var span = source.StartActivity("llm.call"))
        {
            GenAIActivity.RecordMessage(span, "user", "hello", toolName: "search");
            var ev = span!.Events.Single();
            Assert.Equal("gen_ai.user.message", ev.Name);
            Assert.Contains(ev.Tags, t => t.Key == "gen_ai.message.content" && (string?)t.Value == "hello");
            Assert.Contains(ev.Tags, t => t.Key == "gen_ai.tool.name");
        }
    }

    // ---- GenAI cross-SDK divergences, closed -----------------------------

    /// <summary>
    /// Pins the shape the Rust SDK used to get wrong (it emitted a comma-joined
    /// string). A backend filtering by tool cannot do it against a joined
    /// string, and a tool name containing a comma silently became two tools.
    /// </summary>
    [Fact]
    public void SetAttributes_ToolNamesIsAStringArray()
    {
        var source = StartListening(out var listener);
        using (listener)
        using (var span = source.StartActivity("llm.call"))
        {
            GenAIActivity.SetAttributes(span, new GenAIAttributes { ToolNames = new[] { "search", "calc" } });

            var value = span!.GetTagItem("gen_ai.tool.names");
            Assert.IsNotType<string>(value);
            Assert.Equal(new[] { "search", "calc" }, Assert.IsAssignableFrom<IEnumerable<string>>(value));
        }
    }

    /// <summary>
    /// Prompts and tool arguments are the most PII-dense payload this SDK
    /// touches; only the TS SDK used to scrub recorded message content.
    /// </summary>
    [Fact]
    public void RecordMessage_ScrubsContent()
    {
        var source = StartListening(out var listener);
        using (listener)
        using (var span = source.StartActivity("llm.call"))
        {
            GenAIActivity.RecordMessage(span, "user", "mail a@b.com, Authorization: Bearer abc.def-ghi");

            var evt = Assert.Single(span!.Events);
            var content = (string)evt.Tags.Single(t => t.Key == "gen_ai.message.content").Value!;
            Assert.DoesNotContain("a@b.com", content);
            Assert.Contains("Bearer [redacted]", content);

            // No key installed in this test process, so the email is redacted
            // rather than hashed — the fail-safe, not a missed match.
            Assert.Contains("[email:", content);
        }
    }
}
