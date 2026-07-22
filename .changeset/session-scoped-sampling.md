---
'@smooai/observability': minor
---

SMOODEV-2698 (ADR-097 W1+W2): session-scoped browser log sampling, config-served telemetry settings, and the cross-language parity corpus.

- `sampleDecision(id, ratio)` — deterministic FNV-1a 32-bit over the UTF-8 bytes of the session/trace id, so the decision is stable for a page's lifetime and reproducible byte-identically in the Rust/Python/Go/.NET SDKs. Ratio 0.0/1.0 are exact.
- `shouldEmitLog(...)` — one decision point: kill switch → minimum level → warnings/errors always 100% → trace decision inherited where a trace exists → otherwise the session decision. Sampling is per session, never per line, so any trace you can open has 100% of its log lines.
- `loadTelemetrySettings(provider)` / `resolveTelemetrySettings(raw)` — `@smooai/config` public-tier telemetry settings read through an injectable provider seam (the SDK never imports a config client, so it stays usable with no network). Unreachable, malformed, or out-of-range values fall back to the compiled-in ADR-010 defaults, never to "sample everything out".
- `parseTraceparent` / `formatTraceparent` — the first real W3C trace-context implementation in this SDK; strict, rejects all-zero ids.
- `normalizeLevel` — canonical UPPERCASE levels, because ADR-096's error-rate query is case-sensitive.
- `parity/sampling-corpus.json` — 170 committed golden vectors every language SDK asserts against in its own CI lane.
