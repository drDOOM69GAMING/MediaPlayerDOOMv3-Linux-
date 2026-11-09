#!/usr/bin/env bash
# Install build/runtime dependencies for MediaPlayerDOOM.
# Supported first-class targets: CachyOS/Arch and Linux Mint/Debian/Ubuntu.
set -euo pipefail

if command -v pacman >/dev/null 2>&1; then
    echo "Detected Arch/CachyOS (pacman)"
    sudo pacman -S --needed --noconfirm \
        base-devel rust ffmpeg mkvtoolnix-cli cdparanoia \
        libdvdread libdvdnav libdvdcss dvdbackup cdrkit \
        libbluray libaacs \
        alsa-lib openssl libxkbcommon libxkbcommon-x11 wayland libx11 xdg-utils
elif command -v apt >/dev/null 2>&1; then
    echo "Detected Debian/Ubuntu/Mint (apt)"
    sudo apt update
    sudo apt install -y \
        build-essential pkg-config libasound2-dev \
        libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev \
        libssl-dev ffmpeg mkvtoolnix cdparanoia libbluray2 libaacs0 xdg-utils \
        dvdbackup genisoimage \
        xdg-desktop-portal xdg-desktop-portal-gtk
elif command -v dnf >/dev/null 2>&1; then
    echo "Detected Fedora (dnf)"
    sudo dnf install -y gcc gcc-c++ make pkgconf-pkg-config alsa-lib-devel \
        libxkbcommon-devel libxkbcommon-x11-devel wayland-devel libX11-devel \
        openssl-devel ffmpeg mkvtoolnix cdparanoia xdg-utils xdg-desktop-portal
elif command -v zypper >/dev/null 2>&1; then
    echo "Detected openSUSE (zypper)"
    sudo zypper install -y gcc gcc-c++ make pkg-config alsa-devel \
        libxkbcommon-devel libxkbcommon-x11-devel wayland-devel libX11-devel \
        libopenssl-devel ffmpeg mkvtoolnix cdparanoia xdg-utils xdg-desktop-portal
else
    echo "Unknown package manager. Install the packages listed in README.md." >&2
    exit 1
fi

# Make sure a recent Rust toolchain is available (distro rustc is often old).
if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    else
        echo "Installing Rust with rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
fi
echo "Dependencies ready. Now run: ./install.sh"
echo "Note: encrypted DVDs need libdvdcss (Arch: pacman -S libdvdcss;"
echo "      Debian/Ubuntu/Mint: apt install libdvdcss2 from the VideoLAN repo)."
echo "      DUMP TO ISO uses dvdbackup + genisoimage; a CachyOS dvdbackup"
echo "      binary is bundled in packaging/vendor and installed by install.sh."
echo "      Debian names for the DVD stack: libdvdread4 libdvdnav4 libdvdcss2."
