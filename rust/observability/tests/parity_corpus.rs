//! ADR-097 §4 — the Rust lane of the parity corpus.
//!
//! Every SDK (TS, Rust, Python, Go, .NET) asserts against the same
//! `parity/sampling-corpus.json` in its own CI. A language that cannot
//! reproduce a vector fails its build. Documentation claiming parity is not
//! evidence of parity.

use serde_json::Value;
use smooai_observability::sampling::{
    fnv1a32, normalize_level, parse_level, sample_decision, should_emit_log, CanonicalLevel,
    LogSamplingInput,
};
use smooai_observability::telemetry_settings::resolve_telemetry_settings;
use smooai_observability::traceparent::{format_traceparent, parse_traceparent};

/// The corpus lives at the repo root, two levels above this crate.
fn corpus() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../parity/sampling-corpus.json"
    );
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("parity corpus unreadable at {path}: {e}"));
    serde_json::from_str(&raw).expect("parity corpus is not valid JSON")
}

fn section<'a>(c: &'a Value, name: &str) -> &'a Vec<Value> {
    c[name]
        .as_array()
        .unwrap_or_else(|| panic!("corpus section `{name}` missing or not an array"))
}

fn level(s: &str) -> CanonicalLevel {
    parse_level(s).unwrap_or_else(|| panic!("corpus names an unknown canonical level: {s}"))
}

#[test]
fn corpus_is_the_expected_version_and_is_not_empty() {
    let c = corpus();
    assert_eq!(
        c["version"].as_u64(),
        Some(1),
        "corpus schema version changed — re-read parity/README.md before bumping"
    );
    assert!(section(&c, "sampleDecision").len() > 50);
}

#[test]
fn sample_decision_vectors() {
    let c = corpus();
    for v in section(&c, "sampleDecision") {
        let (id, ratio, want) = (
            v["id"].as_str().unwrap(),
            v["ratio"].as_f64().unwrap(),
            v["expected"].as_bool().unwrap(),
        );
        assert_eq!(
            u64::from(fnv1a32(id)),
            v["hash"].as_u64().unwrap(),
            "hash({id:?})"
        );
        assert_eq!(
            sample_decision(id, ratio),
            want,
            "sampleDecision({id:?}, {ratio})"
        );
    }
}

#[test]
fn near_threshold_vectors() {
    let c = corpus();
    for v in section(&c, "sampleDecisionNearThreshold") {
        let (id, ratio, want) = (
            v["id"].as_str().unwrap(),
            v["ratio"].as_f64().unwrap(),
            v["expected"].as_bool().unwrap(),
        );
        let h = fnv1a32(id);
        assert_eq!(u64::from(h), v["hash"].as_u64().unwrap(), "hash({id:?})");
        // The division is exact in binary64, so this is an equality check, not
        // an epsilon one — any drift here means a language got the u32
        // reinterpretation or the divisor wrong.
        assert_eq!(
            f64::from(h) / 4_294_967_296.0,
            v["position"].as_f64().unwrap(),
            "position({id:?})"
        );
        assert_eq!(
            sample_decision(id, ratio),
            want,
            "sampleDecision({id:?}, {ratio})"
        );
    }
}

#[test]
fn non_finite_ratio_fails_open() {
    let c = corpus();
    for v in section(&c, "sampleDecisionNonFiniteRatio") {
        let ratio = match v["ratio"].as_str().unwrap() {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            other => panic!("corpus names an unknown non-finite ratio: {other}"),
        };
        assert_eq!(
            sample_decision(v["id"].as_str().unwrap(), ratio),
            v["expected"].as_bool().unwrap(),
            "{v}"
        );
    }
}

#[test]
fn level_normalization_vectors() {
    let c = corpus();
    for v in section(&c, "levelNormalization") {
        let input = v["input"].as_str().unwrap();
        assert_eq!(
            normalize_level(input).as_str(),
            v["expected"].as_str().unwrap(),
            "normalizeLevel({input:?})"
        );
    }
}

#[test]
fn traceparent_parse_vectors() {
    let c = corpus();
    for v in section(&c, "traceparentParse") {
        let input = v["input"].as_str().unwrap();
        match parse_traceparent(input) {
            None => assert!(
                v["expected"].is_null(),
                "parseTraceparent({input:?}) should have parsed"
            ),
            Some(ctx) => {
                let want = &v["expected"];
                assert!(
                    want.is_object(),
                    "parseTraceparent({input:?}) should have been rejected, got {ctx:?}"
                );
                assert_eq!(ctx.trace_id, want["traceId"].as_str().unwrap(), "{input:?}");
                assert_eq!(ctx.span_id, want["spanId"].as_str().unwrap(), "{input:?}");
                assert_eq!(
                    u64::from(ctx.flags),
                    want["flags"].as_u64().unwrap(),
                    "{input:?}"
                );
                assert_eq!(ctx.sampled, want["sampled"].as_bool().unwrap(), "{input:?}");
            }
        }
    }
}

#[test]
fn traceparent_format_vectors() {
    let c = corpus();
    for v in section(&c, "traceparentFormat") {
        let input = &v["input"];
        let got = format_traceparent(
            input["traceId"].as_str().unwrap(),
            input["spanId"].as_str().unwrap(),
            input.get("flags").and_then(Value::as_i64),
            input.get("sampled").and_then(Value::as_bool),
        );
        assert_eq!(
            got.as_deref(),
            v["expected"].as_str(),
            "formatTraceparent({input})"
        );
    }
}

#[test]
fn settings_resolution_vectors() {
    let c = corpus();
    for v in section(&c, "settingsResolution") {
        let got = resolve_telemetry_settings(&v["input"]);
        let want = &v["expected"];
        assert_eq!(
            got.enabled,
            want["enabled"].as_bool().unwrap(),
            "enabled for {}",
            v["input"]
        );
        assert_eq!(
            got.browser_log_sampling_ratio,
            want["browserLogSamplingRatio"].as_f64().unwrap(),
            "browserLogSamplingRatio for {}",
            v["input"]
        );
        assert_eq!(
            got.minimum_log_level.as_str(),
            want["minimumLogLevel"].as_str().unwrap(),
            "minimumLogLevel for {}",
            v["input"]
        );
        assert_eq!(
            got.trace_sampling_ratio,
            want["traceSamplingRatio"].as_f64().unwrap(),
            "traceSamplingRatio for {}",
            v["input"]
        );
    }
}

#[test]
fn should_emit_log_vectors() {
    let c = corpus();
    for v in section(&c, "shouldEmitLog") {
        let i = &v["input"];
        let got = should_emit_log(&LogSamplingInput {
            level: i["level"].as_str().unwrap(),
            session_id: i["sessionId"].as_str().unwrap(),
            trace_sampled: i.get("traceSampled").and_then(Value::as_bool),
            enabled: i["enabled"].as_bool().unwrap(),
            minimum_level: level(i["minimumLevel"].as_str().unwrap()),
            log_sampling_ratio: i["logSamplingRatio"].as_f64().unwrap(),
        });
        assert_eq!(got, v["expected"].as_bool().unwrap(), "shouldEmitLog({i})");
    }
}
