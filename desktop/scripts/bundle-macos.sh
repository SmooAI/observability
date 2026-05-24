#!/usr/bin/env bash
#
# Bundle Observability Studio as a macOS .app + .dmg.
#
# We deliberately don't depend on dioxus-cli / cargo-bundle / cargo-packager:
# the bundling logic is ~30 lines of shell + an Info.plist template, and
# avoiding extra tooling keeps the GH Actions surface small and the local
# build trivially reproducible.
#
# Usage:
#   scripts/bundle-macos.sh                 # release build + .app + .dmg
#   scripts/bundle-macos.sh --skip-build    # reuse existing release binary
#
# Outputs (relative to the desktop/ root):
#   target/bundle/macos/SmooAI Observability Studio.app
#   target/bundle/macos/SmooAI-Observability-Studio.dmg

set -euo pipefail

# Resolve paths from the script location so the script works whether you're
# in desktop/ or anywhere else.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${DESKTOP_DIR}"

# ---- config ----
APP_NAME="SmooAI Observability Studio"
APP_BUNDLE_ID="ai.smoo.observability.studio"
APP_VERSION="$(grep '^version' crates/observability-studio-app/Cargo.toml | head -1 | cut -d'"' -f2)"
APP_DISPLAY_NAME="${APP_NAME}"
BIN_NAME="observability-studio"
BUNDLE_OUT="target/bundle/macos"
APP_PATH="${BUNDLE_OUT}/${APP_NAME}.app"
DMG_PATH="${BUNDLE_OUT}/SmooAI-Observability-Studio.dmg"
ICON_SRC="assets/icons/icon.icns"

# Allow callers to skip the build step (handy when iterating on the bundle).
if [[ "${1:-}" != "--skip-build" ]]; then
    echo "▸ cargo build --release"
    cargo build --release -p observability-studio-app
fi

BIN_SRC="$(cargo metadata --format-version=1 --no-deps | python3 -c '
import json, sys
m = json.load(sys.stdin)
for t in m["target_directory"], :
    print(t)
')/release/${BIN_NAME}"

if [[ ! -x "${BIN_SRC}" ]]; then
    echo "‼  release binary not found at ${BIN_SRC}" >&2
    exit 1
fi

# ---- build the .app skeleton ----
echo "▸ assembling ${APP_PATH}"
rm -rf "${APP_PATH}"
mkdir -p "${APP_PATH}/Contents/MacOS" "${APP_PATH}/Contents/Resources"

cp "${BIN_SRC}" "${APP_PATH}/Contents/MacOS/${BIN_NAME}"
chmod +x "${APP_PATH}/Contents/MacOS/${BIN_NAME}"

# Icon — generate .icns from the existing 128x128 PNG if .icns isn't already
# in place. iconutil only needs Apple's iconset directory shape.
if [[ ! -f "${ICON_SRC}" ]]; then
    echo "▸ generating ${ICON_SRC} from PNG"
    ICONSET="$(mktemp -d)/icon.iconset"
    mkdir -p "${ICONSET}"
    for size in 16 32 128 256 512; do
        double=$((size * 2))
        sips -z ${size} ${size} assets/icons/icon.png \
             --out "${ICONSET}/icon_${size}x${size}.png" >/dev/null
        sips -z ${double} ${double} assets/icons/icon.png \
             --out "${ICONSET}/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "${ICONSET}" --output "${ICON_SRC}"
fi
cp "${ICON_SRC}" "${APP_PATH}/Contents/Resources/icon.icns"

cat > "${APP_PATH}/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>${BIN_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${APP_BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_DISPLAY_NAME}</string>
    <key>CFBundleVersion</key>
    <string>${APP_VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${APP_VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIconFile</key>
    <string>icon</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <!-- WKWebView needs JIT entitlement; we run via wry's webview. -->
    <key>NSAppTransportSecurity</key>
    <dict>
        <key>NSAllowsArbitraryLoads</key>
        <false/>
    </dict>
</dict>
</plist>
PLIST

# Ad-hoc sign so Gatekeeper at least doesn't flat-out refuse to launch on the
# build machine. Production signing happens in a later phase with a real
# Developer ID cert.
codesign --force --deep -s - "${APP_PATH}" >/dev/null 2>&1 || true

# ---- .dmg ----
echo "▸ packaging ${DMG_PATH}"
rm -f "${DMG_PATH}"
hdiutil create -volname "${APP_NAME}" \
    -srcfolder "${APP_PATH}" \
    -ov -format UDZO "${DMG_PATH}" >/dev/null

echo "✓ bundle ready"
ls -lh "${APP_PATH}" "${DMG_PATH}"
