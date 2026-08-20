//! ADR-097 W1 — config-served telemetry settings. Rust port of
//! `packages/core/src/telemetry-settings.ts`, pinned by
//! `parity/sampling-corpus.json`.
//!
//! These are `@smooai/config` **public-tier**, org-scoped keys. Public tier is
//! mandatory: a browser can never be served secret tier (ADR-075), and the
//! whole point is that changing a key changes every client's behaviour on its
//! next config read. **No secret may ever enter this key set.**
//!
//! FAIL-SAFE IS THE POINT. Unreachable provider, malformed payload,
//! out-of-range value → the compiled-in ADR-010 defaults. Never "sample
//! everything out": a telemetry system that goes silent when its config server
//! hiccups is worse than useless. A caller whose config read *failed* passes
//! `&serde_json::Value::Null` and gets exactly those defaults — which is why
//! this function has no error channel.

use crate::sampling::{parse_level, CanonicalLevel};
use serde_json::Value;

/// The `@smooai/config` public-tier key names this SDK reads.
pub mod keys {
    /// boolean — kill switch. `false` disables ALL telemetry emission.
    pub const ENABLED: &str = "observabilityEnabled";
    /// number 0.0–1.0 — session-scoped browser log sampling ratio.
    pub const BROWSER_LOG_SAMPLING_RATIO: &str = "observabilityBrowserLogSamplingRatio";
    /// string — minimum log level (TRACE|DEBUG|INFO|WARN|ERROR|FATAL).
    pub const MINIMUM_LOG_LEVEL: &str = "observabilityMinimumLogLevel";
    /// number 0.0–1.0 — head-based trace sampling ratio.
    pub const TRACE_SAMPLING_RATIO: &str = "observabilityTraceSamplingRatio";
}

/// Resolved telemetry settings.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetrySettings {
    /// Kill switch. When false nothing is emitted, errors included.
    pub enabled: bool,
    /// Session-scoped browser log sampling ratio. Applied ONCE per session (or
    /// inherited from the trace where one exists) — never per line.
    pub browser_log_sampling_ratio: f64,
    /// Minimum level to emit.
    pub minimum_log_level: CanonicalLevel,
    /// Head-based trace sampling ratio. ADR-010 default: TraceIdRatioBased(0.1).
    pub trace_sampling_ratio: f64,
}

/// Compiled-in ADR-010 defaults. Every failure path lands here.
impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            browser_log_sampling_ratio: 1.0,
            minimum_log_level: CanonicalLevel::Info,
            trace_sampling_ratio: 0.1,
        }
    }
}

/// Ratio coercion.
///
/// Finite number, or decimal numeric string (public config often round-trips
/// values as strings) → clamped into `[0, 1]`; an operator who writes 1.5 means
/// "all". Anything else (missing, NaN, Infinity, boolean, object, unparseable
/// string) → the compiled-in default, never 0.
///
/// The asymmetry is deliberate: a *malformed* value falls back, a *valid but
/// out-of-range* value is clamped. -1 clamps to 0 (telemetry off) because that
/// is an explicit operator value, and 0 is settable anyway.
fn coerce_ratio(raw: Option<&Value>, fallback: f64) -> f64 {
    let n = match raw {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) if !s.trim().is_empty() => {
            s.trim().parse::<f64>().unwrap_or(f64::NAN)
        }
        _ => f64::NAN,
    };
    if !n.is_finite() {
        return fallback;
    }
    n.clamp(0.0, 1.0)
}

fn coerce_bool(raw: Option<&Value>, fallback: bool) -> bool {
    match raw {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => fallback,
        },
        _ => fallback,
    }
}

// `parse_level` (not `normalize_level`) on purpose: normalize maps unknown
// spellings to INFO, which is right for an incoming log line but wrong here — a
// typo'd config value must fall back to the default, not silently reset the floor.
fn coerce_level(raw: Option<&Value>, fallback: CanonicalLevel) -> CanonicalLevel {
    raw.and_then(Value::as_str)
        .and_then(parse_level)
        .unwrap_or(fallback)
}

/// Turn a raw config payload into settings. Total function — never fails,
/// always returns a usable object. Unknown/extra keys are ignored.
pub fn resolve_telemetry_settings(raw: &Value) -> TelemetrySettings {
    let d = TelemetrySettings::default();
    let Some(bag) = raw.as_object() else {
        return d;
    };
    TelemetrySettings {
        enabled: coerce_bool(bag.get(keys::ENABLED), d.enabled),
        browser_log_sampling_ratio: coerce_ratio(
            bag.get(keys::BROWSER_LOG_SAMPLING_RATIO),
            d.browser_log_sampling_ratio,
        ),
        minimum_log_level: coerce_level(bag.get(keys::MINIMUM_LOG_LEVEL), d.minimum_log_level),
        trace_sampling_ratio: coerce_ratio(
            bag.get(keys::TRACE_SAMPLING_RATIO),
            d.trace_sampling_ratio,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unreachable_config_is_defaults_not_all_out() {
        let d = resolve_telemetry_settings(&Value::Null);
        assert!(d.enabled);
        assert_eq!(d.browser_log_sampling_ratio, 1.0);
        assert_eq!(d.trace_sampling_ratio, 0.1);
    }

    #[test]
    fn nan_string_falls_back_rather_than_zeroing() {
        let s =
            resolve_telemetry_settings(&json!({ "observabilityBrowserLogSamplingRatio": "NaN" }));
        assert_eq!(s.browser_log_sampling_ratio, 1.0);
    }
}
