---
'@smooai/observability': patch
---

Every event this SDK has ever sent reported `sdk.version: "0.1.0"`.

`packages/core/src/client.ts` hard-coded `SDK_VERSION = '0.1.0'` while the published package walked from 0.1.0 to 0.19.0. Eighteen minor releases of events landed in the backend labelled with the version of the first one, so "which SDK version produced this event?" — the question the field exists to answer — has been unanswerable for the entire life of the package. The Rust, Python, Go and .NET ports all carried the same frozen constant.

The constant is now derived, not typed. `scripts/sync-versions.mjs` treats `packages/core/package.json` as the single source of truth and writes it into all eleven version-bearing files across the five SDKs — manifests (`Cargo.toml`, `pyproject.toml`, `.csproj`), lockfiles (`Cargo.lock`, `uv.lock`), and the reported-version constants in each language.

It runs in the changesets **`version`** lifecycle, not after publish:

```jsonc
"version": "changeset version && node scripts/sync-versions.mjs"
```

That ordering is the fix, not a detail. The changesets action commits the working tree after `version`, so the synced files land in the release commit and every tag carries the versions it claims. Syncing after `publish` — the pattern in the sibling repos — mutates manifests in a CI workspace that is never committed, which is why those repos need `cargo publish --allow-dirty` to paper over the dirt.

A `--check` mode runs on **every** PR (`pr-checks.yml`, deliberately not path-filtered) and fails on any mismatch. A `TARGETS` row whose pattern matches zero times, or more than once, is a hard error too — a silently-skipped target is the exact failure this script exists to prevent.

One version across languages is the org's existing convention, not a new invention: `@smooai/fetch` is 3.4.1 on npm, crates.io and PyPI alike. The four unreleased SDKs are therefore set to 0.19.0 rather than starting over at 0.1.0.
