# SmooAI Observability Studio

Native desktop client for SmooAI's observability stack — logs, errors, metrics, all from `api.smoo.ai` over M2M `client_credentials`. Built with Dioxus 0.6 on top of the shared [`@smooai/ui`](https://github.com/SmooAI/ui) design system, matching smooblue's stack.

## Install

### macOS

Download the latest `.dmg` from the [GitHub Releases](https://github.com/SmooAI/observability/releases) (look for `studio-v*` tags). The bundles aren't yet code-signed; Gatekeeper will warn on first launch — right-click the .app → Open to bypass.

### Linux

Download the `.AppImage` from the same release. Make it executable and run:

```bash
chmod +x SmooAI-Observability-Studio-x86_64.AppImage
./SmooAI-Observability-Studio-x86_64.AppImage
```

WebKitGTK 4.1 must be installed on the host (`apt install libwebkit2gtk-4.1-0` on Debian/Ubuntu).

### Windows

Download the `.zip` from the release. Unzip and run `observability-studio.exe`.

## Add an org

1. Mint an M2M `client_credentials` pair from the SmooAI dashboard ("Settings → API Keys → New M2M") or via `pnpm --filter @smooai/scripts exec tsx src/create-smoo-m2m-key.ts`.
2. Open the app → **⚙ Settings → Add an org**.
3. Paste the org UUID, label, `client_id`, and `client_secret`. The app verifies the credentials against `https://auth.smoo.ai/token` before committing them to your OS keychain.
4. Switch to the org from the left rail and click 📜 Logs, ⚠ Errors, or 📊 Metrics.

Use **Cmd+K** (Ctrl+K on Win/Linux) to fuzzy-jump between orgs and views.

## Build from source

```bash
cd ~/dev/smooai/observability/desktop
cargo run --release -p observability-studio-app
```

To produce a redistributable bundle for your current platform:

```bash
# macOS  →  target/bundle/macos/SmooAI-Observability-Studio.dmg
./scripts/bundle-macos.sh

# Linux  →  target/bundle/linux/SmooAI-Observability-Studio-x86_64.AppImage
./scripts/bundle-linux.sh

# Windows  →  target\bundle\windows\SmooAI-Observability-Studio-x86_64.zip
pwsh ./scripts/bundle-windows.ps1
```

Each script defaults to building from scratch; pass `--skip-build` to reuse an existing `target/release` binary.

## Architecture

```
desktop/
├── Cargo.toml                                # workspace root
├── Dioxus.toml                               # dx config (when dx is used)
├── crates/
│   ├── observability-studio-app/             # binary: Dioxus components + views
│   │   ├── src/main.rs                       # window bootstrap
│   │   ├── src/lib.rs                        # App component + global key handler
│   │   ├── src/state.rs                      # ActiveSource / RemoteView signals
│   │   ├── src/persistence.rs                # OS-keychain creds + UiState JSON
│   │   ├── src/components/                   # nav rail, status bar, KPI tiles,
│   │   │                                     # time-range picker, stack frames,
│   │   │                                     # Cmd+K palette
│   │   └── src/views/                        # Logs / Errors / Metrics /
│   │                                         # Welcome / Settings dialog
│   ├── observability-studio-client/          # M2M auth + typed api.smoo.ai client
│   │   └── src/api/{logs,errors,metrics}.rs
│   └── observability-studio-theme/           # CSS layer composer
│       └── src/lib.rs                        # smooai_ui::STYLES + app-specific CSS
├── assets/
│   ├── icons/                                # app icon (PNG; .icns generated)
│   └── styles.css                            # app-specific CSS (~1500 lines)
└── scripts/                                  # build + bundle scripts
    ├── bundle-macos.sh
    ├── bundle-linux.sh
    └── bundle-windows.ps1
```

The Rust client + auth layer (`observability-studio-client`) has no Dioxus dependency, so it's drop-in usable from any other Rust app (CLI tools, future TUI, etc.).

## Where the design system comes from

OKLCH tokens, base components, the smoo monogram — all live in [`@smooai/ui`](https://github.com/SmooAI/ui) and are consumed via the `smooai-ui` crate. Don't add `--color-…` tokens to this app's `assets/styles.css`; add them to `@smooai/ui` and bump the version.

## Release process

Bump versions in:

- `desktop/crates/*/Cargo.toml` (workspace pins)

then:

```bash
git tag studio-v0.2.0
git push origin studio-v0.2.0
```

The [`build-desktop.yml`](../.github/workflows/build-desktop.yml) workflow picks up the tag, builds all three platforms, and publishes a GitHub Release with the bundles attached.
