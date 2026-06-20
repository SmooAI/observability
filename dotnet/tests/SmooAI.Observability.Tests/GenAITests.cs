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
}
