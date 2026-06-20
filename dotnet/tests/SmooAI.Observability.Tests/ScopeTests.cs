using SmooAI.Observability;

namespace SmooAI.Observability.Tests;

public class ScopeTests
{
    [Fact]
    public void ApplyToEvent_MergesScopeStateUnderEvent()
    {
        var scope = new Scope();
        scope.SetUser(new UserContext { Id = "u1", OrgId = "o1" });
        scope.SetTag("env", "prod");
        scope.AddBreadcrumb(new Breadcrumb { Category = "nav", Timestamp = 1 });

        var ev = new ObservabilityEvent
        {
            EventId = "e1",
            Tags = new Dictionary<string, string> { ["env"] = "override" },
        };

        var result = scope.ApplyToEvent(ev);

        Assert.Equal("u1", result.User!.Id);
        Assert.Equal("o1", result.User.OrgId);
        // Event tag wins over scope tag.
        Assert.Equal("override", result.Tags!["env"]);
        Assert.Single(result.Breadcrumbs!);
    }

    [Fact]
    public void AddBreadcrumb_CapsAtHundred()
    {
        var scope = new Scope();
        for (var i = 0; i < 150; i++)
        {
            scope.AddBreadcrumb(new Breadcrumb { Category = "c", Timestamp = i });
        }

        var ev = scope.ApplyToEvent(new ObservabilityEvent { EventId = "e" });

        Assert.Equal(100, ev.Breadcrumbs!.Count);
        // Oldest evicted: the first remaining crumb is index 50.
        Assert.Equal(50, ev.Breadcrumbs[0].Timestamp);
    }

    [Fact]
    public void Clone_IsIndependentOfOriginal()
    {
        var scope = new Scope();
        scope.SetTag("a", "1");
        var clone = scope.Clone();
        clone.SetTag("a", "2");
        clone.SetTag("b", "3");

        var original = scope.ApplyToEvent(new ObservabilityEvent { EventId = "e" });
        Assert.Equal("1", original.Tags!["a"]);
        Assert.False(original.Tags.ContainsKey("b"));
    }

    [Fact]
    public void WithScope_IsolatesMutationsAndRestores()
    {
        ObservabilityContext.ResetForTests();
        ObservabilityContext.SetTag("base", "yes");

        ObservabilityContext.WithScope(scope =>
        {
            scope.SetTag("inner", "only");
            var inside = scope.ApplyToEvent(new ObservabilityEvent { EventId = "e" });
            Assert.Equal("yes", inside.Tags!["base"]);
            Assert.Equal("only", inside.Tags["inner"]);
        });

        // After WithScope, the inner tag must not be on the outer scope.
        var after = ObservabilityContext.GetCurrentScope().ApplyToEvent(new ObservabilityEvent { EventId = "e2" });
        Assert.False(after.Tags!.ContainsKey("inner"));
        Assert.Equal("yes", after.Tags["base"]);
    }

    [Fact]
    public async Task WithScopeAsync_FlowsAcrossAwait()
    {
        ObservabilityContext.ResetForTests();
        var observed = false;

        await ObservabilityContext.WithScopeAsync(async scope =>
        {
            scope.SetTag("req", "1");
            await Task.Yield();
            // The scope tag is still present after the await.
            var ev = ObservabilityContext.GetCurrentScope().ApplyToEvent(new ObservabilityEvent { EventId = "e" });
            observed = ev.Tags!.TryGetValue("req", out var v) && v == "1";
        });

        Assert.True(observed);
    }
}
