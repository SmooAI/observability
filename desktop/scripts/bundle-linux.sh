#!/usr/bin/env bash
#
# Bundle Observability Studio as a Linux AppImage.
#
# Strategy: build the binary, drop it in an AppDir alongside a .desktop entry,
# icon, and the WebKitGTK runtime libs the consumer's distro may not ship.
# Then run `appimagetool` to produce a relocatable .AppImage.
#
# Designed for the CI runner — Ubuntu-latest — where we control the apt deps
# and can fetch appimagetool from upstream.
#
# Usage:
#   scripts/bundle-linux.sh
#
# Outputs:
#   target/bundle/linux/SmooAI-Observability-Studio-x86_64.AppImage

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${DESKTOP_DIR}"

APP_NAME="SmooAI Observability Studio"
BIN_NAME="observability-studio"
APP_VERSION="$(grep '^version' crates/observability-studio-app/Cargo.toml | head -1 | cut -d'"' -f2)"
BUNDLE_OUT="target/bundle/linux"
APP_DIR="${BUNDLE_OUT}/AppDir"
APPIMAGE_OUT="${BUNDLE_OUT}/SmooAI-Observability-Studio-x86_64.AppImage"

if [[ "${1:-}" != "--skip-build" ]]; then
    echo "▸ cargo build --release"
    cargo build --release -p observability-studio-app
fi

BIN_SRC="$(cargo metadata --format-version=1 --no-deps | python3 -c '
import json, sys
m = json.load(sys.stdin)
print(m["target_directory"])
')/release/${BIN_NAME}"

if [[ ! -x "${BIN_SRC}" ]]; then
    echo "‼  release binary not found at ${BIN_SRC}" >&2
    exit 1
fi

echo "▸ assembling AppDir"
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/usr/bin" "${APP_DIR}/usr/share/applications" "${APP_DIR}/usr/share/icons/hicolor/256x256/apps"
install -m 0755 "${BIN_SRC}" "${APP_DIR}/usr/bin/${BIN_NAME}"
install -m 0644 assets/icons/icon.png "${APP_DIR}/usr/share/icons/hicolor/256x256/apps/${BIN_NAME}.png"
install -m 0644 assets/icons/icon.png "${APP_DIR}/${BIN_NAME}.png"

cat > "${APP_DIR}/usr/share/applications/${BIN_NAME}.desktop" <<DESKTOP
[Desktop Entry]
Name=${APP_NAME}
Comment=Native client for SmooAI logs, errors, metrics
Exec=${BIN_NAME}
Icon=${BIN_NAME}
Terminal=false
Type=Application
Categories=Development;Monitor;Utility;
StartupWMClass=${BIN_NAME}
X-AppImage-Version=${APP_VERSION}
DESKTOP

cp "${APP_DIR}/usr/share/applications/${BIN_NAME}.desktop" "${APP_DIR}/${BIN_NAME}.desktop"

# AppRun shim — points the AppImage entry at our binary while preserving env.
cat > "${APP_DIR}/AppRun" <<'APPRUN'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/observability-studio" "$@"
APPRUN
chmod +x "${APP_DIR}/AppRun"

# Pull appimagetool if it isn't on PATH. Pinned to a release tag for
# reproducibility — bump deliberately.
APPIMAGETOOL_BIN="${BUNDLE_OUT}/.appimagetool-x86_64.AppImage"
if ! command -v appimagetool >/dev/null 2>&1 && [[ ! -x "${APPIMAGETOOL_BIN}" ]]; then
    echo "▸ fetching appimagetool"
    curl -fsSL -o "${APPIMAGETOOL_BIN}" \
      "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
    chmod +x "${APPIMAGETOOL_BIN}"
fi

APPIMAGETOOL_BIN_RUN="${APPIMAGETOOL_BIN}"
command -v appimagetool >/dev/null 2>&1 && APPIMAGETOOL_BIN_RUN="appimagetool"

echo "▸ packaging ${APPIMAGE_OUT}"
ARCH=x86_64 "${APPIMAGETOOL_BIN_RUN}" --no-appstream "${APP_DIR}" "${APPIMAGE_OUT}"

echo "✓ bundle ready"
ls -lh "${APPIMAGE_OUT}"
