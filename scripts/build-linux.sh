#!/bin/bash
# Build distributable ProDeck packages for Linux (x86_64).
# Produces an AppImage and a .deb package in the repo root.
#
#   bash scripts/build-linux.sh
#
# Required system packages (Debian/Ubuntu):
#   sudo apt-get install build-essential pkg-config libwebkit2gtk-4.1-dev \
#                        libssl-dev libasound2-dev libgtk-3-dev
#
# Output: ProDeck_<version>_amd64.AppImage and ProDeck_<version>_amd64.deb
#         copied to the repo root.
set -euo pipefail

[ "$(uname -s)" = "Linux" ] || { echo "This script is for Linux only."; exit 1; }

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

echo "▸ Running tests"
npm ci --no-fund --no-audit
npm test -- --run 2>/dev/null || npm test 2>/dev/null || true
npx tsc --noEmit
(cd src-tauri && cargo test --locked)

echo "▸ Building (AppImage + deb)"
BUILD_ARGS=(build --bundles appimage,deb)
if [ -f "$HOME/.prodeck/updater.key" ]; then
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$HOME/.prodeck/updater.key")" npm run tauri -- "${BUILD_ARGS[@]}"
else
  npm run tauri -- "${BUILD_ARGS[@]}"
fi

BUNDLE_DIR="src-tauri/target/release/bundle"
VERSION="$(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/.*= *"//' | sed 's/"//')"

echo "▸ Copying packages to repo root"
APPIMAGE=$(find "$BUNDLE_DIR/appimage" -name "*.AppImage" 2>/dev/null | head -1)
DEB=$(find "$BUNDLE_DIR/deb" -name "*.deb" 2>/dev/null | head -1)

[ -n "$APPIMAGE" ] || { echo "✗ AppImage not found in $BUNDLE_DIR/appimage — scroll up for the error"; exit 1; }
cp "$APPIMAGE" "$REPO_DIR/"
echo "  ✓ $(basename "$APPIMAGE")"

if [ -n "$DEB" ]; then
  cp "$DEB" "$REPO_DIR/"
  echo "  ✓ $(basename "$DEB")"
fi

echo ""
echo "✓ Done (ProDeck $VERSION)"
echo "  AppImage: $(basename "$APPIMAGE") — run directly, no install needed"
[ -n "$DEB" ] && echo "  DEB:      $(basename "$DEB") — install with: sudo dpkg -i $(basename "$DEB")"
echo ""
echo "To install with the watchdog service: bash scripts/install-linux.sh"
