import asyncio

from smooai_observability.scope import get_current_scope, with_scope
from smooai_observability.types import ObservabilityEvent, Sdk, User


def _event() -> ObservabilityEvent:
    return ObservabilityEvent(
        event_id="id",
        timestamp=1,
        level="error",
        sdk=Sdk("@smooai/observability", "0.1.0", "python"),
    )


def test_apply_to_event_merges_scope():
    with with_scope() as scope:
        scope.set_user(User(id="u1", org_id="o1"))
        scope.set_tag("env", "test")
        scope.set_context("runtime", {"py": "3"})
        scope.add_breadcrumb("custom", "did a thing")
        ev = scope.apply_to_event(_event())
    assert ev.user.id == "u1"
    assert ev.user.org_id == "o1"
    assert ev.tags == {"env": "test"}
    assert ev.contexts == {"runtime": {"py": "3"}}
    assert len(ev.breadcrumbs) == 1
    assert ev.breadcrumbs[0].message == "did a thing"


def test_event_user_overrides_scope_user():
    with with_scope() as scope:
        scope.set_user(User(id="scope-user", org_id="o1"))
        ev = _event()
        ev.user = User(id="event-user")
        merged = scope.apply_to_event(ev)
    assert merged.user.id == "event-user"  # event wins
    assert merged.user.org_id == "o1"  # filled from scope


def test_breadcrumb_buffer_caps_at_100():
    with with_scope() as scope:
        for i in range(150):
            scope.add_breadcrumb("custom", f"crumb-{i}")
        ev = scope.apply_to_event(_event())
    assert len(ev.breadcrumbs) == 100
    assert ev.breadcrumbs[0].message == "crumb-50"  # oldest dropped
    assert ev.breadcrumbs[-1].message == "crumb-149"


def test_with_scope_is_isolated():
    outer = get_current_scope()
    outer.set_tag("base", "1")
    with with_scope() as inner:
        inner.set_tag("inner-only", "x")
    # Inner mutation must not leak to the outer scope.
    ev = outer.apply_to_event(_event())
    assert ev.tags.get("base") == "1"
    assert "inner-only" not in ev.tags


def test_async_scope_isolation():
    results: dict[str, dict] = {}

    async def task(name: str, value: str):
        with with_scope() as scope:
            scope.set_tag("who", value)
            await asyncio.sleep(0.01)  # force interleaving
            ev = scope.apply_to_event(_event())
            results[name] = ev.tags

    async def main():
        await asyncio.gather(task("a", "alice"), task("b", "bob"))

    asyncio.run(main())
    assert results["a"]["who"] == "alice"
    assert results["b"]["who"] == "bob"
