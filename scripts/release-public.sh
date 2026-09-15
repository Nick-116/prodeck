#!/bin/bash
# Publish a public ProDeck release: the drag-to-install DMG PLUS the signed
# auto-update artifact, as one GitHub Release. Every installed copy checks
# .../releases/latest/download/latest.json at launch, so publishing here is
# what pushes the new version to everyone who downloaded the app.
#
#   scripts/release-public.sh "What changed"      # release notes (optional)
#
# Steps: bump "version" in src-tauri/tauri.conf.json first (tag = v<version>).
#
# Needs:
#   ~/.prodeck/public-updater.key   the PUBLIC-release signing key (passwordless).
#                                   Its public half is baked into tauri.conf.json;
#                                   the app refuses updates signed by anything else.
#                                   BACK IT UP — losing it strands every install on
#                                   its current version until they re-download.
#   gh (authenticated for the release repo)
#
# The Windows installer is BUILT by GitHub Actions (this is a Mac) and SIGNED
# here, because the minisign key never leaves this machine. Set SKIP_WINDOWS=1
# to publish macOS-only — Windows copies then simply see no update, which is
# the same as before this existed. Needs the release commit pushed: CI builds
# what's on the remote, not your working copy.
#
# One signed app is used for BOTH the DMG and the updater tarball, so what a
# new downloader gets and what an updater installs are byte-identical.
set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

REPO="${PUBLIC_REPO:-whiteoakmedia/prodeck}"
KEY_PATH="${PUBLIC_UPDATER_KEY:-$HOME/.prodeck/public-updater.key}"
NOTES="${1:-}"
OUT="$REPO_DIR/release"
BUNDLE_DIR="src-tauri/target/universal-apple-darwin/release/bundle/macos"
APP="$BUNDLE_DIR/ProDeck.app"

[ -f "$KEY_PATH" ] || { echo "✗ signing key not found at $KEY_PATH"; exit 1; }
command -v gh >/dev/null || { echo "✗ gh CLI required (brew install gh; gh auth login)"; exit 1; }
VERSION="$(node -p "require('./src-tauri/tauri.conf.json').version")"
TAG="v$VERSION"
EXPECT_PUB="$(tr -d '\n' < "${KEY_PATH}.pub" 2>/dev/null || true)"
CONF_PUB="$(node -p "require('./src-tauri/tauri.conf.json').plugins.updater.pubkey")"
if [ -n "$EXPECT_PUB" ] && [ "$EXPECT_PUB" != "$CONF_PUB" ]; then
  echo "✗ tauri.conf.json pubkey does not match $KEY_PATH.pub — installed apps would reject this update."
  exit 1
fi
# Cargo.lock records the crate's own version, and bumping tauri.conf.json does
# not touch it — so a release commit can carry Cargo.toml at the new version and
# Cargo.lock at the old one. Nothing on macOS notices; the Windows CI runs
# `cargo test --locked` and fails outright. Refuse to publish out of sync.
LOCK_V="$(awk '/^name = "prodeck"$/{getline; gsub(/[^0-9.]/,"",$0); print; exit}' src-tauri/Cargo.lock)"
if [ "$LOCK_V" != "$VERSION" ]; then
  echo "✗ src-tauri/Cargo.lock says $LOCK_V but this release is $VERSION."
  echo "  Run:  (cd src-tauri && cargo check --quiet)  then commit Cargo.lock."
  exit 1
fi

# CI builds the commit that is on the remote, and that .exe is what gets signed
# and shipped. Releasing from an unpushed tree would publish an installer built
# from different source than the DMG beside it.
WIN_BRANCH=""
if [ "${SKIP_WINDOWS:-0}" != "1" ]; then
  WIN_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
  [ -z "$(git status --porcelain)" ] || {
    echo "✗ uncommitted changes — commit and push before releasing (or SKIP_WINDOWS=1)."; exit 1; }
  git fetch -q origin "$WIN_BRANCH"
  if [ "$(git rev-parse HEAD)" != "$(git rev-parse "origin/$WIN_BRANCH")" ]; then
    echo "✗ $WIN_BRANCH is not pushed — CI would build a different commit than this DMG."
    echo "  Run:  git push origin $WIN_BRANCH   (or SKIP_WINDOWS=1 for a macOS-only release)"
    exit 1
  fi
fi

echo "▸ Releasing ProDeck $TAG to github.com/$REPO"

echo "▸ Building universal app"
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
rm -rf "$APP"
# No personal signing identity, no tauri updater artifacts — we sign the app
# once below and tar it ourselves so DMG and updater carry the same bytes.
npm run tauri -- build --target universal-apple-darwin --bundles app \
  --config '{"bundle":{"createUpdaterArtifacts":false,"macOS":{"signingIdentity":null}}}'
[ -d "$APP" ] || { echo "✗ build produced no app — scroll up"; exit 1; }

BIN="$APP/Contents/MacOS/prodeck"
echo "▸ Verifying universal + self-contained"
ARCHS="$(lipo -archs "$BIN")"
case "$ARCHS" in *x86_64*arm64*|*arm64*x86_64*) ;; *) echo "✗ not universal: $ARCHS"; exit 1 ;; esac
for arch in arm64 x86_64; do
  if otool -arch "$arch" -L "$BIN" | grep -E '^\s' | grep -vE '/usr/lib/|/System/'; then
    echo "✗ $arch slice links a non-system library — would crash on other Macs"; exit 1
  fi
done
BUILT_V="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")"
[ "$BUILT_V" = "$VERSION" ] || { echo "✗ built $BUILT_V but tauri.conf.json says $VERSION"; exit 1; }
echo "  ✓ $ARCHS · system libraries only · v$BUILT_V"

echo "▸ Signing (ad-hoc, no hardened runtime — opens on any Mac after one Open Anyway)"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

rm -rf "$OUT"; mkdir -p "$OUT"

echo "▸ Updater artifact"
tar -czf "$OUT/ProDeck.app.tar.gz" -C "$BUNDLE_DIR" ProDeck.app
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_PATH")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  npm run tauri -- signer sign "$OUT/ProDeck.app.tar.gz" >/dev/null
[ -f "$OUT/ProDeck.app.tar.gz.sig" ] || { echo "✗ signing produced no .sig"; exit 1; }
SIG="$(tr -d '\n' < "$OUT/ProDeck.app.tar.gz.sig")"

echo "▸ DMG"
STAGING="$(mktemp -d)"; cp -R "$APP" "$STAGING/"; ln -s /Applications "$STAGING/Applications"
hdiutil create -volname "ProDeck" -srcfolder "$STAGING" -ov -format UDZO "$OUT/ProDeck.dmg" >/dev/null
rm -rf "$STAGING"

WIN_EXE=""; WIN_SIG=""
if [ -n "$WIN_BRANCH" ]; then
  echo "▸ Windows installer (GitHub Actions builds it, this Mac signs it)"
  SHA="$(git rev-parse HEAD)"
  gh workflow run windows.yml --repo "$REPO" --ref "$WIN_BRANCH" >/dev/null
  # Wait for the dispatch to show up as a run FOR THIS COMMIT. Matching on sha
  # rather than "the newest run" keeps a concurrent PR build from being mistaken
  # for ours and shipping the wrong bytes.
  RUN=""
  for _ in $(seq 1 20); do
    sleep 6
    RUN="$(gh run list --workflow=windows.yml --repo "$REPO" --event workflow_dispatch \
             --limit 10 --json databaseId,headSha \
             --jq "[.[] | select(.headSha==\"$SHA\")] | first | .databaseId // empty")"
    [ -n "$RUN" ] && break
  done
  [ -n "$RUN" ] || { echo "✗ no Windows run appeared for $SHA — check the Actions tab"; exit 1; }
  echo "  run $RUN — building (~7 min)"
  gh run watch "$RUN" --repo "$REPO" --exit-status >/dev/null || {
    echo "✗ Windows build failed: https://github.com/$REPO/actions/runs/$RUN"; exit 1; }
  rm -rf "$OUT/win"
  gh run download "$RUN" --repo "$REPO" -n ProDeck-Windows-x64 -D "$OUT/win"
  WIN_EXE="$(ls "$OUT"/win/*.exe 2>/dev/null | head -1)"
  [ -n "$WIN_EXE" ] || { echo "✗ the Windows run produced no .exe"; exit 1; }
  # The installer carries its version in the filename; a stale cached artifact
  # would otherwise be signed and published as if it were this release.
  case "$(basename "$WIN_EXE")" in
    *"_${VERSION}_"*) ;;
    *) echo "✗ $(basename "$WIN_EXE") is not version $VERSION"; exit 1 ;;
  esac
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_PATH")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
    npm run tauri -- signer sign "$WIN_EXE" >/dev/null
  [ -f "$WIN_EXE.sig" ] || { echo "✗ signing produced no .sig for the installer"; exit 1; }
  WIN_SIG="$(tr -d '\n' < "$WIN_EXE.sig")"
  echo "  ✓ $(basename "$WIN_EXE") signed"
fi

echo "▸ latest.json"
ASSET_URL="https://github.com/$REPO/releases/download/$TAG/ProDeck.app.tar.gz"
WIN_URL=""
[ -n "$WIN_EXE" ] && WIN_URL="https://github.com/$REPO/releases/download/$TAG/$(basename "$WIN_EXE")"
node -e '
const [version, notes, sig, url, winSig, winUrl] = process.argv.slice(1);
const p = { signature: sig, url };
const platforms = { "darwin-aarch64": p, "darwin-x86_64": p, "darwin-universal": p };
// Absent rather than empty: the updater treats an unknown platform as "you are
// up to date", but a present entry with a blank signature is a hard error on
// every Windows launch.
if (winUrl) platforms["windows-x86_64"] = { signature: winSig, url: winUrl };
process.stdout.write(JSON.stringify({
  version, notes, pub_date: new Date().toISOString(), platforms,
}, null, 2) + "\n");
' "$VERSION" "${NOTES:-ProDeck $TAG}" "$SIG" "$ASSET_URL" "$WIN_SIG" "$WIN_URL" > "$OUT/latest.json"

# Only advertise Windows when a signed installer is actually attached — a
# download section pointing at a file that isn't there is worse than silence.
WIN_BODY=""
if [ -n "$WIN_EXE" ]; then
  WIN_BODY="
## Windows
Download **$(basename "$WIN_EXE")** and run it. It installs for the current user only, so there's no admin prompt. Installed copies update themselves from this release automatically.

Windows build and packaging by [@jpeters0](https://github.com/jpeters0)."
fi

echo "▸ Publishing $TAG"
ASSETS=("$OUT/ProDeck.dmg" "$OUT/ProDeck.app.tar.gz" "$OUT/latest.json")
[ -n "$WIN_EXE" ] && ASSETS+=("$WIN_EXE")
BODY="${NOTES:-ProDeck $TAG}

## Install
1. Download **ProDeck.dmg**, open it, drag **ProDeck** to Applications.
2. First open only (macOS Sequoia/Tahoe): double-click → **System Settings → Privacy & Security** → **Open Anyway**.

Runs on Intel and Apple Silicon (macOS 10.15+). Installed copies update themselves from this release automatically.
$WIN_BODY"
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh release upload "$TAG" "${ASSETS[@]}" --repo "$REPO" --clobber
  gh release edit "$TAG" --repo "$REPO" --latest --notes "$BODY" >/dev/null
else
  gh release create "$TAG" "${ASSETS[@]}" --repo "$REPO" --title "ProDeck $TAG" --notes "$BODY" --latest
fi

echo ""
echo "✓ Published https://github.com/$REPO/releases/tag/$TAG"
echo "  Feed: https://github.com/$REPO/releases/latest/download/latest.json"
echo "  Every installed copy will offer $TAG at its next launch."
if [ -n "$WIN_EXE" ]; then
  echo "  Windows: $(basename "$WIN_EXE") signed and in the feed."
else
  echo "  Windows: SKIPPED — Windows copies will see no update for $TAG."
fi
