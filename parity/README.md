# Parity corpus (ADR-097 §4)

`sampling-corpus.json` is the contract between the five `@smooai/observability`
SDKs: TypeScript (`packages/core`, the reference implementation), Rust
(`rust/`), Python (`python/`), Go (`go/`), .NET (`dotnet/`).

Every language asserts against **this same file** in its own CI lane. A language
that cannot reproduce a vector fails its build. This exists because ADR-007/009
documented cross-language behaviour that was never implemented in any of them —
documentation claiming parity is not evidence of parity.

Regenerate with:

```bash
node parity/generate-corpus.mjs
```

The generator re-derives the hash vectors from an implementation written
independently of the SDK's, and self-checks against the published FNV-1a-32
vectors before writing anything. Non-hash expectations are hand-authored from
the specs (W3C trace-context, ADR-096 level casing, ADR-010 defaults).

## The hash — everything a porter needs

**FNV-1a, 32-bit**, over the **UTF-8 bytes** of the id:

```
h = 0x811c9dc5                      # FNV offset basis, 32-bit
for each byte b of utf8(id):
    h = h XOR b                     # XOR first — this is FNV-*1a*, not FNV-1
    h = (h * 0x01000193) mod 2^32   # FNV prime, wrapping 32-bit multiply
```

`h` is **unsigned** 32-bit. There is no endianness to get wrong — it is a
byte-at-a-time fold, not a word load. Languages with signed-only 32-bit integers
(Go `int32`, C# `int`, Java) must reinterpret as unsigned before the division.

Decision:

```
if ratio is not finite  -> IN    (fail open — see below)
if ratio <= 0.0         -> OUT   (exact, taken before any float math)
if ratio >= 1.0         -> IN    (exact, taken before any float math)
else                    -> (h / 2^32) < ratio        # STRICT less-than
```

`h / 2^32` is **exact** in IEEE-754 binary64: `h < 2^32` needs at most 32
mantissa bits and 2^32 is a power of two, so the division is a pure exponent
adjustment with no rounding. Divide by the literal `4294967296.0` in binary64
(`f64`, `double`, Python `float`) and every language produces the identical
double.

The 0.0 / 1.0 branches come first specifically so ratio 1.0 can never drop a
session and ratio 0.0 can never keep one, whatever the hash happens to be.

Non-finite ratio (NaN, ±Inf) fails **open**. `x < NaN` is false in every
IEEE-754 language, so the naive path would silently sample _everything_ out —
the exact "telemetry goes quiet when config hiccups" failure ADR-097 forbids.

### Porting traps this corpus is built to catch

- folding UTF-16 code units or code points instead of UTF-8 bytes (see the emoji
  and CJK ids)
- signed 32-bit arithmetic leaking a negative into the division
- a wrapping multiply implemented as `(h * prime) & 0xffffffff` in a float
  language — the product exceeds 2^53 and is silently wrong (this bit the
  generator itself; use `Math.imul` / 16-bit halves / native u32)
- `<=` instead of `<` at the threshold (see `sampleDecisionNearThreshold`)
- lowercase level output (ADR-096's error-rate query is
  `level IN ('ERROR','FATAL')` and is **case-sensitive**)
- trimming the id before hashing — whitespace is significant

## File format

Top-level keys, each an array of vectors:

| Key                            | Vector shape                                                          | Asserts                                                                                        |
| ------------------------------ | --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `sampleDecision`               | `{ id, ratio, hash, expected }`                                       | `fnv1a32(id) == hash` and `sampleDecision(id, ratio) == expected`                              |
| `sampleDecisionNearThreshold`  | `{ id, ratio, hash, position, expected }`                             | same, for ids hashing within 1e-4 of the 0.5 threshold                                         |
| `sampleDecisionNonFiniteRatio` | `{ id, ratio: "NaN"\|"Infinity"\|"-Infinity", expected }`             | JSON has no non-finite literals, so the ratio is a **string** the port maps to its float value |
| `levelNormalization`           | `{ input, expected }`                                                 | `normalizeLevel(input) == expected` (canonical UPPERCASE)                                      |
| `traceparentParse`             | `{ input, expected: {traceId,spanId,flags,sampled}\|null }`           | strict W3C parse; `null` means the header must be rejected                                     |
| `traceparentFormat`            | `{ input: {traceId,spanId,flags?,sampled?}, expected: string\|null }` | format; `null` means the port must refuse to emit                                              |
| `settingsResolution`           | `{ input, expected }`                                                 | raw config payload → resolved settings, including every fail-safe path                         |
| `shouldEmitLog`                | `{ input, expected }`                                                 | the full decision: kill switch → min level → WARN+ always-on → trace inheritance → session     |

A `why` field appears on some vectors — it is documentation for humans, not part
of the assertion. `hash` and `position` are likewise redundant with `expected`;
they exist so a failing port can tell _where_ it diverged (hash vs comparison).

The current file pins **170 vectors** at `version: 1`. Bump `version` on any
breaking change to the format and update every language lane in the same PR.

## Config keys (ADR-097 §2)

The settings vectors use the `@smooai/config` **public-tier** key names the SDK
reads. Public tier is a hard requirement: a browser can never be served secret
tier (ADR-075). No secret may ever enter this key set.

| Key                                    | Type           | Default  |
| -------------------------------------- | -------------- | -------- |
| `observabilityEnabled`                 | boolean        | `true`   |
| `observabilityBrowserLogSamplingRatio` | number 0.0–1.0 | `1.0`    |
| `observabilityMinimumLogLevel`         | level string   | `"INFO"` |
| `observabilityTraceSamplingRatio`      | number 0.0–1.0 | `0.1`    |

Registering these keys in a consumer's `.smooai-config` schema happens in the
**monorepo**, not here — this repo only defines the names, the defaults, and the
coercion rules. The SDK never imports `@smooai/config`; the host app supplies
values through the injectable `TelemetrySettingsProvider` seam so the SDK stays
usable in a test or a browser bundle with no network.
