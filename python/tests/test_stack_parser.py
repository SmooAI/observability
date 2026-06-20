from smooai_observability.stack_parser import parse_traceback
from smooai_observability.types import StackFrame


def _raise_chain():
    def inner():
        raise ValueError("inner boom")

    def outer():
        inner()

    outer()


def test_parse_traceback_innermost_first():
    try:
        _raise_chain()
    except ValueError as err:
        frames = parse_traceback(err.__traceback__)
    assert frames, "expected at least one frame"
    # Innermost frame should be `inner` (the raising function).
    assert frames[0].function == "inner"
    assert frames[0].lineno is not None
    # This test file is application code.
    assert frames[0].in_app is True


def test_parse_traceback_none():
    assert parse_traceback(None) == []


def test_frame_fields_present():
    try:
        raise RuntimeError("x")
    except RuntimeError as err:
        frames = parse_traceback(err.__traceback__)
    f = frames[0]
    assert isinstance(f, StackFrame)
    assert f.module.endswith(".py")
