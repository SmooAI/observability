using SmooAI.Observability;

namespace SmooAI.Observability.Tests;

public class StackParserTests
{
    [Fact]
    public void Parse_ProducesFramesForThrownException()
    {
        Exception captured;
        try
        {
            ThrowDeep();
            throw new InvalidOperationException("unreachable");
        }
        catch (Exception ex)
        {
            captured = ex;
        }

        var frames = StackParser.Parse(captured);

        Assert.NotEmpty(frames);
        // The throwing method should appear somewhere in the trace.
        Assert.Contains(frames, f => f.Function is "ThrowDeep" or "Inner");
    }

    [Fact]
    public void Parse_TagsApplicationFramesInApp()
    {
        Exception captured;
        try
        {
            throw new InvalidOperationException("boom");
        }
        catch (Exception ex)
        {
            captured = ex;
        }

        var frames = StackParser.Parse(captured);
        // Test code lives in the SmooAI.Observability.Tests namespace, which is
        // not in the non-app prefix list, so its frames are in-app.
        Assert.Contains(frames, f => f.InApp == true);
    }

    [Fact]
    public void Parse_NeverThrownException_ReturnsEmpty()
    {
        var frames = StackParser.Parse(new InvalidOperationException("not thrown"));
        Assert.Empty(frames);
    }

    [Fact]
    public void DropSdkFrames_RemovesLeadingSdkFrames()
    {
        var frames = new List<StackFrame>
        {
            new() { Module = "SmooAI.Observability.Client", Function = "Capture", InApp = false },
            new() { Module = "MyApp.Service", Function = "DoWork", InApp = true },
        };

        var result = StackParser.DropSdkFrames(frames);

        Assert.Single(result);
        Assert.Equal("MyApp.Service", result[0].Module);
    }

    [Fact]
    public void DropSdkFrames_KeepsAllWhenNoSdkFramesLead()
    {
        var frames = new List<StackFrame>
        {
            new() { Module = "MyApp.Service", Function = "DoWork", InApp = true },
        };
        var result = StackParser.DropSdkFrames(frames);
        Assert.Single(result);
    }

    private static void ThrowDeep() => Inner();

    private static void Inner() => throw new InvalidOperationException("deep boom");
}
