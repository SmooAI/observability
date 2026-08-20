"""ADR-097 §4 — the Python lane of the parity corpus.

Every SDK (TS, Rust, Python, Go, .NET) asserts against the same
``parity/sampling-corpus.json`` in its own CI. A language that cannot reproduce
a vector fails its build. Documentation claiming parity is not evidence of
parity.
"""

from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any

import pytest

from smooai_observability.sampling import (
    CanonicalLevel,
    LogSamplingInput,
    fnv1a32,
    normalize_level,
    sample_decision,
    should_emit_log,
)
from smooai_observability.telemetry_settings import resolve_telemetry_settings
from smooai_observability.traceparent import format_traceparent, parse_traceparent

# tests/ -> python/ -> repo root
CORPUS_PATH = Path(__file__).resolve().parents[2] / "parity" / "sampling-corpus.json"
CORPUS: dict[str, Any] = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))

_NON_FINITE = {"NaN": math.nan, "Infinity": math.inf, "-Infinity": -math.inf}


def section(name: str) -> list[dict[str, Any]]:
    vectors = CORPUS[name]
    assert isinstance(vectors, list) and vectors, f"corpus section `{name}` missing or empty"
    return vectors


def test_corpus_is_the_expected_version_and_is_not_empty():
    assert CORPUS["version"] == 1, "corpus schema version changed — re-read parity/README.md before bumping"
    assert len(section("sampleDecision")) > 50


@pytest.mark.parametrize("v", section("sampleDecision"))
def test_sample_decision(v):
    assert fnv1a32(v["id"]) == v["hash"]
    assert sample_decision(v["id"], v["ratio"]) is v["expected"]


@pytest.mark.parametrize("v", section("sampleDecisionNearThreshold"))
def test_sample_decision_near_threshold(v):
    h = fnv1a32(v["id"])
    assert h == v["hash"]
    # The division is exact in binary64, so this is an equality check, not an
    # epsilon one — drift here means a language got the u32 reinterpretation or
    # the divisor wrong.
    assert h / 4294967296.0 == v["position"]
    assert sample_decision(v["id"], v["ratio"]) is v["expected"]


@pytest.mark.parametrize("v", section("sampleDecisionNonFiniteRatio"))
def test_non_finite_ratio_fails_open(v):
    assert sample_decision(v["id"], _NON_FINITE[v["ratio"]]) is v["expected"]


@pytest.mark.parametrize("v", section("levelNormalization"))
def test_level_normalization(v):
    assert normalize_level(v["input"]).value == v["expected"]


@pytest.mark.parametrize("v", section("traceparentParse"))
def test_traceparent_parse(v):
    got = parse_traceparent(v["input"])
    want = v["expected"]
    if want is None:
        assert got is None
    else:
        assert got is not None
        assert got.trace_id == want["traceId"]
        assert got.span_id == want["spanId"]
        assert got.flags == want["flags"]
        assert got.sampled is want["sampled"]


@pytest.mark.parametrize("v", section("traceparentFormat"))
def test_traceparent_format(v):
    i = v["input"]
    assert format_traceparent(i["traceId"], i["spanId"], i.get("flags"), i.get("sampled")) == v["expected"]


@pytest.mark.parametrize("v", section("settingsResolution"))
def test_settings_resolution(v):
    got = resolve_telemetry_settings(v["input"])
    want = v["expected"]
    assert got.enabled is want["enabled"]
    assert got.browser_log_sampling_ratio == want["browserLogSamplingRatio"]
    assert got.minimum_log_level.value == want["minimumLogLevel"]
    assert got.trace_sampling_ratio == want["traceSamplingRatio"]


@pytest.mark.parametrize("v", section("shouldEmitLog"))
def test_should_emit_log(v):
    i = v["input"]
    got = should_emit_log(
        LogSamplingInput(
            level=i["level"],
            session_id=i["sessionId"],
            trace_sampled=i.get("traceSampled"),
            enabled=i["enabled"],
            minimum_level=CanonicalLevel(i["minimumLevel"]),
            log_sampling_ratio=i["logSamplingRatio"],
        )
    )
    assert got is v["expected"]
