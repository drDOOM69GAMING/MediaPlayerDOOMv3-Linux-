#!/usr/bin/env bash
# Install MediaPlayerDOOM for the current user, including its icon and desktop
# entry. Use PREFIX=/usr with sudo to install system-wide.
set -euo pipefail
cd "$(dirname "$0")"

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor/512x512/apps"

echo "Building release..."
cargo build --release

echo "Installing to $PREFIX"
install -Dm755 target/release/mediaplayerofdoom "$BIN/mediaplayerofdoom"
install -Dm644 packaging/mediaplayerofdoom.desktop "$APPS/mediaplayerofdoom.desktop"
install -Dm644 assets/mediaplayerofdoom.png "$ICONS/mediaplayerofdoom.png"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
    kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
fi

echo "Done. Launch it from your application menu, or run: mediaplayerofdoom"
echo "If $BIN is not on PATH, add it: export PATH=\"$BIN:\$PATH\""
