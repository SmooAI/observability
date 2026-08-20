# Releasing

Five SDKs, four registries, **one version number**.

That version lives in `packages/core/package.json` and nowhere else. Every other
version-bearing file in this repo — three manifests, two lockfiles, five
reported-version constants, two Go `require` lines, one `go.work` replace — is
derived from it by `scripts/sync-versions.mjs`, which runs inside the changesets
`version` lifecycle so the synced files land in the release commit. `--check`
mode runs on every PR and fails on drift.

One version across languages is the org's existing convention, not a local
invention: `@smooai/fetch` is `3.4.1` on npm, crates.io and PyPI alike.

## TypeScript — automatic

Merge a changeset to `main`. `release.yml` opens (and auto-merges) a
"🦋 New version release" PR, then publishes `@smooai/observability` to npm.
Nothing to do by hand.

## Rust, Python, .NET, Go — tag-driven

`publish.yml` is dormant until a language-prefixed tag is pushed. Pushing to a
branch publishes nothing.

| tag                  | goes to                                 | secret needed                 |
| -------------------- | --------------------------------------- | ----------------------------- |
| `rust-v<semver>`     | crates.io (`smooai-observability`)      | `SMOOAI_CARGO_REGISTRY_TOKEN` |
| `python-v<semver>`   | PyPI (`smooai-observability`)           | `SMOOAI_PYPI_TOKEN`           |
| `dotnet-v<semver>`   | NuGet (`SmooAI.Observability`)          | `SMOOAI_NUGET_API_KEY`        |
| `go/v<semver>`       | pkg.go.dev (`…/observability/go`)       | none — the module proxy       |
| `go/fiber/v<semver>` | pkg.go.dev (`…/observability/go/fiber`) | none                          |
| `go/gin/v<semver>`   | pkg.go.dev (`…/observability/go/gin`)   | none                          |

All three secrets are **org-level and already present** — nothing to create.
Each publish job also refuses to start if its credential resolves to the empty
string, so a missing or invisible secret fails on a bare runner instead of after
a clean package at the upload step.

Every job depends on a `verify` gate that asserts (a) all version-bearing files
agree with `packages/core/package.json` and (b) the tag names that same version.
A mistyped tag fails before any toolchain boots — which matters, because
**crates.io, PyPI and NuGet publishes are irreversible**. A version can be
yanked; it can never be replaced.

### The steps

1. Let the npm release land first, so `packages/core/package.json` carries the
   version you are about to publish everywhere else.
2. Confirm the tree agrees:
    ```bash
    node scripts/sync-versions.mjs --check
    ```
3. Dry-run each language from a `main` checkout at that commit:
    ```bash
    (cd rust   && cargo publish --locked -p smooai-observability --dry-run)
    (cd python && uv build --wheel --sdist && uvx twine check dist/*)
    (cd dotnet && dotnet pack src/SmooAI.Observability/SmooAI.Observability.csproj -c Release -o nupkg)
    (cd go     && bash ../scripts/check-go-modules.sh)
    ```
    Or push the button without a tag: **Actions → Publish SDKs → Run workflow**,
    pick a language, tick `dry_run`. Same gates, no upload.
4. Tag and push. `V` is the version from step 1, with no `v` prefix of its own:
    ```bash
    V=0.19.2
    git tag "rust-v$V"   && git push origin "rust-v$V"
    git tag "python-v$V" && git push origin "python-v$V"
    git tag "dotnet-v$V" && git push origin "dotnet-v$V"
    ```

### Go: order matters

`go/fiber` and `go/gin` are separate modules whose `go.mod` files `require` the
core at the exact version being released. They cannot resolve until it is on the
proxy, so:

```bash
V=0.19.2
git tag "go/v$V" && git push origin "go/v$V"
# wait for the `go` job's proxy-warm step to report success (~1 min)
git tag "go/fiber/v$V" && git push origin "go/fiber/v$V"
git tag "go/gin/v$V"   && git push origin "go/gin/v$V"
```

There is no `replace` directive in any published `go.mod`, deliberately: Go
honours `replace` only in the main module, so one there is invisible to
consumers and the module resolves nothing. Local builds get the sibling source
from `go/go.work`, which is not published. `scripts/check-go-modules.sh` runs on
every PR and again at publish to keep it that way.

### Verify afterwards

```bash
curl -s https://crates.io/api/v1/crates/smooai-observability | head -c 200
curl -s https://pypi.org/pypi/smooai-observability/json | head -c 200
curl -s https://api.nuget.org/v3-flatcontainer/smooai.observability/index.json
GOPROXY=proxy.golang.org go list -m github.com/SmooAI/observability/go@v$V
GOPROXY=proxy.golang.org go list -m github.com/SmooAI/observability/go/fiber@v$V
```

## Desktop studio

Separate track, separate version: push a `studio-v*` tag and `build-desktop.yml`
bundles macOS / Linux / Windows into a GitHub Release. Bundles are **unsigned**;
signing and notarization are still to do. The studio is not on any package
registry, so its version does not participate in the SDK lockstep.

## If a publish half-fails

Registries are independent — a failed NuGet push does not roll back crates.io.
Re-push the same tag (delete it locally and remotely first) after fixing the
cause; the jobs are idempotent up to the registry (`dotnet nuget push` uses
`--skip-duplicate`, `cargo publish` errors on an existing version rather than
overwriting). If a bad version reached a registry, publish a fixed patch — do
not try to reuse the number.
