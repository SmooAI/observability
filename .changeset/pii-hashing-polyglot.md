---
'@smooai/observability': minor
---

TypeScript, Go, Python and .NET: hash PII instead of leaking it, matching the Rust SDK.

All four SDKs scrubbed **credentials only** — `Bearer`, `password=`,
`token`/`api_key`/`secret=`, `sk-…` — while their module docs claimed "PII
scrubbing". Emails, phone numbers and street addresses passed through to the
backend untouched. Rust fixed this in #82; this brings the other four to parity
with byte-identical output.

Personal identifiers are now **hashed, not dropped**: `a@b.com` →
`[email:9f2a41c8]`. The type prefix stays visible, so "are these two spans the
same person?" stays answerable while nothing reversible is stored. The hash is
**HMAC-SHA256, keyed** — not a bare digest, which a rainbow table reverses in
seconds for a space as small as email addresses — and the org id is mixed into
the message, so identical PII hashes differently in different orgs. Phone
numbers normalize to digits and emails to lowercase before hashing, so
`(415) 555-0142` and `415-555-0142` correlate.

Credentials are still **dropped**, never hashed, and are matched first: a hash of
a live token is a token oracle, and PII inside a secret (`token=a@b.com`) goes
with the secret. With no key configured (`SMOOAI_OBSERVABILITY_PII_HASH_KEY`, or
the per-SDK setter), personal identifiers are fully redacted (`[email:redacted]`)
rather than hashed under a guessable one — fail closed, never fail open.

New API, same shape in every SDK. The org-less entry points keep working
unchanged (they hash under the empty org salt):

- TypeScript: `setPiiHashKey`, `piiToken`, `scrubStringForOrg`,
  `scrubHeadersForOrg`, `PiiKind` — now exported from the package entry
- Go: `SetPiiHashKey`, `PiiToken`, `ScrubStringForOrg`, `ScrubHeadersForOrg`,
  `PiiKind`, `BootstrapEnv.PiiHashKey`
- Python: `set_pii_hash_key`, `pii_token`, `scrub_string_for_org`,
  `scrub_headers_for_org`, `PiiKind`, `BootstrapEnv.pii_hash_key`
- .NET: `Pii.SetPiiHashKey`, `Pii.PiiToken`, `Pii.ScrubStringForOrg`,
  `Pii.ScrubHeadersForOrg`, `PiiKind`, `BootstrapEnv.PiiHashKey`

`piiToken(kind, raw, orgId)` is the search seam: hash a typed query term the same
way and match the stored token.

⚠️ **The key is load-bearing — rotate never.** Rotating it silently forks
correlation with every hash already stored. Supply it once at startup; the
setters are set-once and refuse a second key.

The TypeScript SDK ships a small synchronous SHA-256/HMAC (`hmac-sha256.ts`)
rather than taking a dependency: `scrubString` is sync and runs in the browser
bundle, where `node:crypto` is unavailable and WebCrypto is async-only. It is
pinned by the RFC 4231 and FIPS 180-4 vectors.
