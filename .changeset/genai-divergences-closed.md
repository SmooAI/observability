---
'@smooai/observability': patch
---

Rust, Python, Go and .NET: close the two GenAI divergences the README's ledger recorded.

The TypeScript SDK is unchanged — this is the other four catching up to it, recorded here because one npm version is the whole SDK family's changelog.

**`gen_ai.tool.names` is a string array in Rust too.** Rust emitted `names.join(",")` where TypeScript, Python, Go and .NET all emitted a string array. Two consequences: a backend filtering spans by tool could not do it against a Rust service's spans at all, and a tool name containing a comma silently became two tools. Now `Value::Array(Array::String(…))`, matching the other four and the OTel spec's array-valued attribute.

**Recorded GenAI message content is PII-scrubbed in all five.** `recordGenAIMessage` scrubbed content in TypeScript only; the Rust, Python, Go and .NET ports wrote the raw string onto the span event. Prompts and tool arguments are the single most PII-dense payload this SDK can touch — raw emails, phone numbers, addresses and pasted credentials routinely appear in them — so every port now routes content through its own `scrubString` before the event is added. That drops credentials and hashes personal identifiers per-org, exactly as the TS reference does.

Each fix ships with a span-level test in its own language (the assertion needs a real exported span, not a string), so the ledger row is now backed by CI rather than by prose.

Note the scrub uses the **org-less** entry point in every SDK, so hashes are salted with the empty org: there is no org id in hand at this call site. Same as TypeScript. If an org id ever reaches here, switch to the `ForOrg` variant.
