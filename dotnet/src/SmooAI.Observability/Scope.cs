namespace SmooAI.Observability;

/// <summary>
/// Mutable per-context state (user, tags, contexts, breadcrumbs) merged into
/// every captured event. Mirrors the TS <c>Scope</c> class. The "current" scope
/// is tracked with <see cref="AsyncLocal{T}"/> so it flows across async/await
/// and is isolated per request / per logical call chain — the .NET analogue of
/// the TS module-level scope stack.
/// </summary>
public sealed class Scope
{
    private const int MaxBreadcrumbs = 100;

    private UserContext? _user;
    private readonly Dictionary<string, string> _tags = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Dictionary<string, object?>> _contexts = new(StringComparer.Ordinal);
    private readonly List<Breadcrumb> _breadcrumbs = new();
    private readonly object _gate = new();

    /// <summary>Set the user/org/session context on this scope.</summary>
    public void SetUser(UserContext? user)
    {
        lock (_gate)
        {
            _user = user;
        }
    }

    /// <summary>Set a single tag.</summary>
    public void SetTag(string key, string value)
    {
        ArgumentNullException.ThrowIfNull(key);
        lock (_gate)
        {
            _tags[key] = value;
        }
    }

    /// <summary>Set a free-form context bag (e.g. "os", "device").</summary>
    public void SetContext(string key, Dictionary<string, object?> context)
    {
        ArgumentNullException.ThrowIfNull(key);
        ArgumentNullException.ThrowIfNull(context);
        lock (_gate)
        {
            _contexts[key] = context;
        }
    }

    /// <summary>Append a breadcrumb, evicting the oldest beyond the 100-item cap.</summary>
    public void AddBreadcrumb(Breadcrumb breadcrumb)
    {
        ArgumentNullException.ThrowIfNull(breadcrumb);
        lock (_gate)
        {
            _breadcrumbs.Add(breadcrumb);
            if (_breadcrumbs.Count > MaxBreadcrumbs)
            {
                _breadcrumbs.RemoveRange(0, _breadcrumbs.Count - MaxBreadcrumbs);
            }
        }
    }

    /// <summary>Drop all breadcrumbs from this scope.</summary>
    public void ClearBreadcrumbs()
    {
        lock (_gate)
        {
            _breadcrumbs.Clear();
        }
    }

    /// <summary>
    /// Merge this scope's state into an event. Event-level values win over scope
    /// values, matching the TS spread order (<c>{ ...scope, ...event }</c>).
    /// </summary>
    public ObservabilityEvent ApplyToEvent(ObservabilityEvent ev)
    {
        ArgumentNullException.ThrowIfNull(ev);
        lock (_gate)
        {
            // User: shallow-merge scope under event.
            if (_user is not null || ev.User is not null)
            {
                ev.User = new UserContext
                {
                    Id = ev.User?.Id ?? _user?.Id,
                    OrgId = ev.User?.OrgId ?? _user?.OrgId,
                    SessionId = ev.User?.SessionId ?? _user?.SessionId,
                };
            }

            // Tags: scope first, event overrides.
            if (_tags.Count > 0 || ev.Tags is not null)
            {
                var merged = new Dictionary<string, string>(_tags, StringComparer.Ordinal);
                if (ev.Tags is not null)
                {
                    foreach (var (k, v) in ev.Tags)
                    {
                        merged[k] = v;
                    }
                }
                ev.Tags = merged;
            }

            // Contexts: scope first, event overrides.
            if (_contexts.Count > 0 || ev.Contexts is not null)
            {
                var merged = new Dictionary<string, Dictionary<string, object?>>(_contexts, StringComparer.Ordinal);
                if (ev.Contexts is not null)
                {
                    foreach (var (k, v) in ev.Contexts)
                    {
                        merged[k] = v;
                    }
                }
                ev.Contexts = merged;
            }

            // Breadcrumbs: scope first, then any event-supplied crumbs.
            if (_breadcrumbs.Count > 0 || ev.Breadcrumbs is not null)
            {
                var merged = new List<Breadcrumb>(_breadcrumbs);
                if (ev.Breadcrumbs is not null)
                {
                    merged.AddRange(ev.Breadcrumbs);
                }
                ev.Breadcrumbs = merged;
            }
        }
        return ev;
    }

    /// <summary>Deep-ish clone for <see cref="ObservabilityContext.WithScope"/>.</summary>
    public Scope Clone()
    {
        var clone = new Scope();
        lock (_gate)
        {
            clone._user = _user is null
                ? null
                : new UserContext { Id = _user.Id, OrgId = _user.OrgId, SessionId = _user.SessionId };
            foreach (var (k, v) in _tags)
            {
                clone._tags[k] = v;
            }
            foreach (var (k, v) in _contexts)
            {
                clone._contexts[k] = v;
            }
            clone._breadcrumbs.AddRange(_breadcrumbs);
        }
        return clone;
    }
}

/// <summary>
/// Ambient scope management. Holds the current <see cref="Scope"/> in an
/// <see cref="AsyncLocal{T}"/> and exposes the static convenience surface
/// (<see cref="SetUser"/>, <see cref="SetTag"/>, etc.) that the
/// <see cref="ObservabilityClient"/> reads from.
/// </summary>
public static class ObservabilityContext
{
    private static readonly AsyncLocal<Scope?> CurrentScope = new();

    /// <summary>The scope for the current logical call context. Created lazily.</summary>
    public static Scope GetCurrentScope()
    {
        var scope = CurrentScope.Value;
        if (scope is null)
        {
            scope = new Scope();
            CurrentScope.Value = scope;
        }
        return scope;
    }

    /// <summary>Set the user on the current scope.</summary>
    public static void SetUser(UserContext? user) => GetCurrentScope().SetUser(user);

    /// <summary>Set a tag on the current scope.</summary>
    public static void SetTag(string key, string value) => GetCurrentScope().SetTag(key, value);

    /// <summary>Set a context bag on the current scope.</summary>
    public static void SetContext(string key, Dictionary<string, object?> context) =>
        GetCurrentScope().SetContext(key, context);

    /// <summary>Add a breadcrumb to the current scope.</summary>
    public static void AddBreadcrumb(string category, string? message = null, Dictionary<string, object?>? data = null, Level level = Level.Info) =>
        GetCurrentScope().AddBreadcrumb(new Breadcrumb
        {
            Category = category,
            Message = message,
            Data = data,
            Level = level,
            Timestamp = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        });

    /// <summary>
    /// Run <paramref name="action"/> with a forked clone of the current scope.
    /// Mutations inside the action are isolated and discarded on exit. The
    /// previous scope is restored even if the action throws.
    /// </summary>
    public static void WithScope(Action<Scope> action)
    {
        ArgumentNullException.ThrowIfNull(action);
        var previous = CurrentScope.Value;
        var next = GetCurrentScope().Clone();
        CurrentScope.Value = next;
        try
        {
            action(next);
        }
        finally
        {
            CurrentScope.Value = previous;
        }
    }

    /// <summary>Async-aware overload of <see cref="WithScope(Action{Scope})"/>.</summary>
    public static async Task WithScopeAsync(Func<Scope, Task> action)
    {
        ArgumentNullException.ThrowIfNull(action);
        var previous = CurrentScope.Value;
        var next = GetCurrentScope().Clone();
        CurrentScope.Value = next;
        try
        {
            await action(next).ConfigureAwait(false);
        }
        finally
        {
            CurrentScope.Value = previous;
        }
    }

    /// <summary>Test seam — reset the ambient scope.</summary>
    internal static void ResetForTests() => CurrentScope.Value = null;
}
