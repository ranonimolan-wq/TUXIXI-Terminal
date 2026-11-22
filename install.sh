#!/bin/sh
# TUXIXI installer - downloads the latest release binary from GitHub
# and installs it into ~/.local. No root, no Python, no distro guessing.

set -e

REPO="ranonimolan-wq/TUXIXI-Terminal"
BIN_DIR="$HOME/.local/bin"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
DESKTOP_DIR="$HOME/.local/share/applications"

command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 || {
    echo "error: curl or wget is required"; exit 1; }

fetch() { # fetch <url> <output>
    if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"
    else wget -qO "$2" "$1"; fi
}

TAG=$(fetch "https://api.github.com/repos/$REPO/releases/latest" /dev/stdout 2>/dev/null \
    | grep -o '"tag_name": *"[^"]*"' | cut -d'"' -f4)
[ -n "$TAG" ] || { echo "error: could not determine the latest release"; exit 1; }

ASSET="tuxixi-${TAG}-x86_64-linux.tar.gz"
URL="https://github.com/$REPO/releases/download/$TAG/$ASSET"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "Downloading TUXIXI $TAG ..."
fetch "$URL" "$TMP/$ASSET" || { echo "error: download failed ($URL)"; exit 1; }
tar -xzf "$TMP/$ASSET" -C "$TMP"

mkdir -p "$BIN_DIR" "$ICON_DIR" "$DESKTOP_DIR"
install -m 755 "$TMP/tuxixi" "$BIN_DIR/tuxixi"
[ -f "$TMP/tuxixi-512.png" ] && install -m 644 "$TMP/tuxixi-512.png" "$ICON_DIR/tuxixi-512.png"

cat > "$DESKTOP_DIR/org.tuxixi.Terminal.desktop" <<EOF
[Desktop Entry]
Name=TUXIXI Terminal
Comment=A simple, customizable terminal emulator for Linux
Exec=$BIN_DIR/tuxixi
Icon=tuxixi-512
Terminal=false
Type=Application
Categories=System;TerminalEmulator;
EOF

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "note: add TUXIXI to your PATH:  export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

echo "Installed TUXIXI $TAG to $BIN_DIR/tuxixi"
