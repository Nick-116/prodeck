#!/bin/bash
# Install ProDeck on Linux and hand the process to a systemd user service
# so launchd owns it and relaunches it on any crash.
#
# Installs the binary to /usr/local/bin/prodeck, then enables a systemd
# user service (prodeck.service) that starts it at login and restarts it
# after any crash. A deliberate Quit stays quit.
#
# Must be run after scripts/build-linux.sh (or npm run tauri build).
set -euo pipefail

[ "$(uname -s)" = "Linux" ] || { echo "This script is for Linux only."; exit 1; }

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_DIR="$REPO_DIR/src-tauri/target/release/bundle"
SERVICE_DEPLOY="$REPO_DIR/deploy/systemd/prodeck.service"
INSTALL_BIN="/usr/local/bin/prodeck"
SERVICE_DIR="$HOME/.config/systemd/user"
SERVICE_FILE="$SERVICE_DIR/prodeck.service"

# ---------------------------------------------------------------- find binary
# Prefer the AppImage (self-contained, no install step); fall back to the
# raw binary from a plain `cargo build`.
APPIMAGE=$(find "$BUNDLE_DIR/appimage" -name "*.AppImage" 2>/dev/null | head -1 || true)
RAW_BIN="$REPO_DIR/src-tauri/target/release/prodeck"

if [ -n "$APPIMAGE" ]; then
  SRC="$APPIMAGE"
  echo "installing AppImage: $(basename "$APPIMAGE")"
elif [ -f "$RAW_BIN" ]; then
  SRC="$RAW_BIN"
  echo "installing binary: prodeck"
else
  echo "No built binary found. Run scripts/build-linux.sh first."
  exit 1
fi

VERSION="$(grep '^version' "$REPO_DIR/src-tauri/Cargo.toml" | head -1 | sed 's/.*= *"//' | sed 's/"//')"
echo "installing ProDeck $VERSION to $INSTALL_BIN"

# ---------------------------------------------------------------- stop service
# Take the watchdog out of the picture so nothing respawns mid-install.
systemctl --user stop prodeck 2>/dev/null || true
systemctl --user disable prodeck 2>/dev/null || true

# Wait for the running process to exit (up to 10 s), then insist.
running() { pgrep -x prodeck >/dev/null 2>&1; }
for _ in $(seq 1 20); do
  running || break
  sleep 0.5
done
if running; then
  pkill -x prodeck 2>/dev/null || true
  sleep 1
fi

# ---------------------------------------------------------------- install binary
sudo install -m 755 "$SRC" "$INSTALL_BIN"
echo "  ✓ installed to $INSTALL_BIN"

# ---------------------------------------------------------------- install service
mkdir -p "$SERVICE_DIR"
if [ -f "$SERVICE_DEPLOY" ]; then
  # Use the repo's unit file, substituting the installed binary path.
  sed "s|ExecStart=.*|ExecStart=$INSTALL_BIN|" "$SERVICE_DEPLOY" > "$SERVICE_FILE"
else
  # Generate a minimal unit file if the repo copy is missing.
  cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=ProDeck booth hub
After=network.target

[Service]
ExecStart=$INSTALL_BIN
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF
fi

# Enable lingering so the user service starts at boot even without a desktop login
# (needed for headless booth machines). Errors here are non-fatal — it just means
# ProDeck won't start until the user logs in.
loginctl enable-linger "$USER" 2>/dev/null || true

systemctl --user daemon-reload
systemctl --user enable prodeck
systemctl --user start prodeck
echo "  ✓ prodeck.service enabled and started"

echo ""
echo "ProDeck $VERSION running under prodeck.service"
echo "  status:  systemctl --user status prodeck"
echo "  logs:    journalctl --user -u prodeck -f"
echo "  stop:    systemctl --user stop prodeck"
