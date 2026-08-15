---
'@smooai/observability': minor
---

Rust: PII is now hashed rather than passed through. `pii::scrub_string` handled
credentials only — `Bearer`, `password=`, `token`/`api_key`/`secret=`, `sk-…` —
while the module doc claimed PII scrubbing, so an email or phone in a message,
breadcrumb or GenAI tool argument reached the wire intact.

Emails, phone numbers and street addresses are now detected and replaced with a
keyed token: `a@b.com` → `[email:9f2a41c8]`. HMAC-SHA256, not a bare digest —
those values are a small enumerable space a rainbow table reverses in seconds —
and the org id is mixed into the message so identical PII hashes differently in
different orgs. The type prefix stays visible, which keeps "are these two spans
the same person?" answerable while storing nothing reversible.

Credentials are still **dropped**, never hashed: a hash of a live token is a
token oracle. With no key configured (`SMOOAI_OBSERVABILITY_PII_HASH_KEY`, or
`pii::set_pii_hash_key`), personal identifiers are fully redacted rather than
hashed under a guessable key.

New: `pii::scrub_string_for_org`, `pii::scrub_headers_for_org`, `pii::pii_token`,
`pii::PiiKind`, `pii::set_pii_hash_key`, `BootstrapEnv::pii_hash_key`.
`scrub_string` / `scrub_headers` keep their signatures and now scrub personal
identifiers too, under the empty org salt.
