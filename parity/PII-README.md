# PII corpus (ADR-097 §4)

`pii-corpus.json` is the second contract between the five
`@smooai/observability` SDKs, alongside [`sampling-corpus.json`](README.md). It
pins the exact bytes of a **PII token** — the `[email:02ea437f]` handle that
replaces a personal identifier in a scrubbed string.

Every language asserts against **this same file** in its own CI lane. Before it
existed, the same seven tuples were **typed out five times**, once per language:

| SDK        | Loader                                                | Implementation                           |
| ---------- | ----------------------------------------------------- | ---------------------------------------- |
| TypeScript | `packages/core/src/__tests__/pii.test.ts`             | `packages/core/src/pii.ts`               |
| Rust       | `rust/observability/src/pii.rs` (`mod tests`)         | `rust/observability/src/pii.rs`          |
| Python     | `python/tests/test_pii.py`                            | `python/src/smooai_observability/pii.py` |
| Go         | `go/pii_test.go`                                      | `go/pii.go`                              |
| .NET       | `dotnet/tests/SmooAI.Observability.Tests/PiiTests.cs` | `dotnet/src/SmooAI.Observability/Pii.cs` |

Hand-copied literals detect a divergence in a **value** — but only in a value
somebody remembered to copy. Nothing detected a divergence in the **set**: add a
vector in one language and the other four stay silently unaudited. The Rust and
.NET loaders live inside the test file next to the implementation rather than in
a separate integration test, because the token function is deliberately
crate-private / `internal` in both.

Regenerate with:

```bash
node parity/generate-pii-corpus.mjs
```

The generator derives tokens from the spec below using `node:crypto`, and
**self-checks against the seven tuples that shipped in all five SDKs** before
writing anything. If the derivation disagrees with what shipped, it refuses to
write — the generator cannot silently redefine the contract.

## The token — everything a porter needs

The hash is **HMAC-SHA256, keyed** — not a bare digest. Emails and phone numbers
are a small enumerable space that a rainbow table reverses in seconds.

```
normalized = normalize(kind, raw)
message    = utf8(orgId) || 0x00 || utf8(kind) || 0x00 || utf8(normalized)
mac        = HMAC-SHA256(key, message)
token      = "[" + kind + ":" + lowercase_hex(mac)[0..8] + "]"
```

Three things in that message are load-bearing:

- **`orgId` is the per-org salt.** The same email hashes differently in different
  orgs, so nothing correlates across tenants — matching the ClickHouse
  org-isolation posture. An empty org id is a real salt value, not "skip the
  salt".
- **`kind` is in the message** so a phone and an address that normalize alike
  cannot collide.
- **The `0x00` delimiters** stop framing ambiguity: without them
  `("ab", "c")` and `("a", "bc")` would hash identically.

Normalization, per kind:

| kind      | normalize                                                | so that                                                            |
| --------- | -------------------------------------------------------- | ------------------------------------------------------------------ |
| `email`   | trim, then lowercase                                     | `A@B.COM ` correlates with `a@b.com`                               |
| `phone`   | keep ASCII digits only                                   | `(415) 555-0142`, `415-555-0142` and `415.555.0142` all correlate  |
| `address` | collapse runs of whitespace to one space, then lowercase | `1600  Pennsylvania   Ave` correlates with `1600 PENNSYLVANIA AVE` |

**With no key installed the token is `[<kind>:redacted]`** — fail safe. An
unkeyed digest of an email is worse than useless: it looks like protection and
reverses instantly.

### Porting traps this corpus is built to catch

- **Dropping the NUL delimiters** — the tokens still look right and still
  correlate within one language, so nothing local catches it
- **Truncating the MAC bytes instead of the hex** — `mac[0..8]` as bytes is 16
  hex chars, not 8
- **Trimming or normalizing more than the spec says**: plus-addressing is NOT
  stripped (`a+tag@b.com` is a different person than `a@b.com`), and a phone's
  country code IS digits, so `+1 415-555-0142` is a different hash than
  `415-555-0142`
- **Whitespace that is not a space** — `\t` and `\n` collapse in an address the
  same way a space does, which a naive `replace(/ +/g, ' ')` gets wrong
- **Non-UTF-8 message bytes** — the message is UTF-8, and lowercasing is
  Unicode-aware
- **Hashing credentials** — credentials are DROPPED, never hashed. A hash of a
  live token is still an oracle you can test candidates against. Scrub ordering
  (credentials first) is enforced by each SDK's own tests, not by this corpus.

## File format

| Key                 | Vector shape                          | Asserts                                                       |
| ------------------- | ------------------------------------- | ------------------------------------------------------------- |
| `tokenWithKey`      | `{ kind, raw, orgId, expected, why }` | `tokenWithKey(kind, raw, orgId, hash.key) == expected`        |
| `tokenWithOtherKey` | `{ kind, raw, orgId, expected }`      | same inputs under `hash.otherKey` — proves the token is keyed |
| `tokenWithoutKey`   | `{ kind, raw, orgId, expected }`      | no key installed → `[<kind>:redacted]`                        |

`hash.key` and `hash.otherKey` are the test keys, committed on purpose: they are
not secrets, they exist so five languages can agree on bytes. Each lane asserts
its local key constant equals the corpus's, so rotating a key reads as "wrong
key" rather than "the hash broke".

A `why` field appears on some vectors — documentation for humans, not part of
the assertion.

Bump `version` on any breaking change to the format and update every language
lane in the same PR.
