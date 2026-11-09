#!/usr/bin/env bash
# Install MediaPlayerDOOM for the current user, including its icon and desktop
# entry. Use PREFIX=/usr with sudo to install system-wide.
set -euo pipefail
cd "$(dirname "$0")"

# rustup installs cargo here; non-login shells may not have it on PATH yet.
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

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

# A Linux ELF binary cannot carry a file-manager icon (that is a Windows PE
# feature). Provide an icon-bearing launcher in the project folder and, when
# available, on the desktop instead.
make_launcher() {
    local file="$1" exec_path="$2" icon_path="$3"
    cat > "$file" <<EOF
[Desktop Entry]
Type=Application
Name=MediaPlayerDOOM
GenericName=Media Player
Comment=Retro DOOM-styled music and video player
Exec=$exec_path %F
Icon=$icon_path
Terminal=false
Categories=AudioVideo;Audio;Video;Player;
MimeType=audio/mpeg;audio/flac;audio/x-flac;audio/x-wav;audio/ogg;audio/opus;audio/mp4;audio/x-m4a;video/mp4;video/x-matroska;video/x-msvideo;video/webm;
Keywords=music;audio;video;player;doom;retro;mp3;flac;mkv;
StartupNotify=true
StartupWMClass=mediaplayerofdoom
EOF
    chmod +x "$file"
    gio set "$file" metadata::trusted true >/dev/null 2>&1 || true
}

make_launcher "$PWD/MediaPlayerDOOM.desktop" "$PWD/target/release/mediaplayerofdoom" "$PWD/assets/mediaplayerofdoom.png"

DESKTOP_DIR="${XDG_DESKTOP_DIR:-$HOME/Desktop}"
if [ -d "$DESKTOP_DIR" ]; then
    make_launcher "$DESKTOP_DIR/MediaPlayerDOOM.desktop" "$BIN/mediaplayerofdoom" "$ICONS/mediaplayerofdoom.png"
    echo "Desktop shortcut: $DESKTOP_DIR/MediaPlayerDOOM.desktop"
fi

echo "Done. Launch it from your application menu, from MediaPlayerDOOM.desktop, or run: mediaplayerofdoom"
echo "If $BIN is not on PATH, add it: export PATH=\"$BIN:\$PATH\""
