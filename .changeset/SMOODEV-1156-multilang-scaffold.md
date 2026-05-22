---
'@smooai/observability': minor
---

SMOODEV-1155 + SMOODEV-1156–1159: scaffold multi-language SDK subdirs and add OTel GenAI semantic-conventions helpers.

- New scaffolds under `dotnet/`, `go/`, `python/`, `rust/` mirroring the layout of `~/dev/smooai/logger/`. Each is a placeholder package manifest + README pointing at the canonical TS reference and its tracking ticket.
- New `setGenAIAttributes(span, attrs)` + `recordGenAIMessage(span, role, content)` helpers for emitting the OTel `gen_ai.*` attribute family on LLM and agent spans. Backs the upcoming LLM Observability dashboard (SMOODEV-1160).
