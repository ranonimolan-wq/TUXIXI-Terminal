#!/bin/sh
# ==========================================================
#  TUXIXI TERMINAL INSTALLER
#
#  Downloads the latest release binary from GitHub and
#  installs it into ~/.local. No root, no Python, no
#  distro guessing:
#
#    curl -fsSL https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main/install.sh | bash
# ==========================================================

set -e

REPO="ranonimolan-wq/TUXIXI-Terminal"
BIN_DIR="$HOME/.local/bin"
USER_DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
ICON_DIR="$USER_DATA_DIR/icons/hicolor/512x512/apps"
DESKTOP_DIR="$USER_DATA_DIR/applications"

# ----------------------------------------------------------
# 0. Helpers
# ----------------------------------------------------------

need_cmd() { command -v "$1" >/dev/null 2>&1; }

if [ -t 1 ]; then
    BOLD=$(printf '\033[1m')
    DIM=$(printf '\033[2m')
    RED=$(printf '\033[31m')
    LIME=$(printf '\033[32m')
    AMBER=$(printf '\033[33m')
    PURPLE=$(printf '\033[35m')
    CYAN=$(printf '\033[36m')
    RESET=$(printf '\033[0m')
else
    BOLD="" DIM="" RED="" LIME="" AMBER="" PURPLE="" CYAN="" RESET=""
fi

fetch() { # fetch <url> <output>
    if need_cmd curl; then curl -fsSL "$1" -o "$2"
    elif need_cmd wget; then wget -qO "$2" "$1"
    else
        echo "  ${RED}error:${RESET} curl or wget is required." >&2
        exit 1
    fi
}

step() { echo "  ${DIM}•${RESET} $1"; }
ok()   { echo "  ${LIME}✓${RESET} $1"; }

# ----------------------------------------------------------
# 1. Download
# ----------------------------------------------------------

echo ""
echo "  ${BOLD}${PURPLE}◆ TUXIXI TERMINAL INSTALLER${RESET}"
echo "  ${DIM}──────────────────────────────────${RESET}"
echo ""

# Stable redirect: GitHub forwards this to the newest release asset,
# so no GitHub API call (and no rate limit) is involved.
URL="https://github.com/$REPO/releases/latest/download/tuxixi-x86_64-linux.tar.gz"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

step "Downloading the latest TUXIXI release..."
fetch "$URL" "$TMP/tuxixi.tar.gz" || {
    echo "  ${RED}error:${RESET} download failed ($URL)" >&2
    exit 1
}
ok "Downloaded"

step "Extracting..."
tar -xzf "$TMP/tuxixi.tar.gz" -C "$TMP"
ok "Extracted"

# ----------------------------------------------------------
# 2. Install
# ----------------------------------------------------------

mkdir -p "$BIN_DIR" "$ICON_DIR" "$DESKTOP_DIR"

step "Installing binary to $BIN_DIR ..."
install -m 755 "$TMP/tuxixi" "$BIN_DIR/tuxixi"
ok "Binary installed"

step "Installing application icon..."
[ -f "$TMP/tuxixi-512.png" ] && install -m 644 "$TMP/tuxixi-512.png" "$ICON_DIR/tuxixi-512.png"
ok "Icon installed ($ICON_DIR/tuxixi-512.png)"

step "Creating desktop entry..."
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
chmod 644 "$DESKTOP_DIR/org.tuxixi.Terminal.desktop" 2>/dev/null || true
ok "Desktop entry created ($DESKTOP_DIR/org.tuxixi.Terminal.desktop)"

if need_cmd update-desktop-database; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi
if need_cmd gtk-update-icon-cache && [ -d "$USER_DATA_DIR/icons/hicolor" ]; then
    gtk-update-icon-cache -f -t "$USER_DATA_DIR/icons/hicolor" 2>/dev/null || true
fi

# ----------------------------------------------------------
# 3. Finish
# ----------------------------------------------------------

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) PATH_NOTE=1 ;;
esac

echo ""
echo "  ${DIM}──────────────────────────────────${RESET}"
echo "  ${BOLD}${PURPLE}◆ TUXIXI INSTALLED!${RESET}"
echo "  ${DIM}──────────────────────────────────${RESET}"
echo ""
echo "  Binary    : ${CYAN}$BIN_DIR/tuxixi${RESET}"
echo "  Icon      : ${CYAN}$ICON_DIR/tuxixi-512.png${RESET}"
echo "  Desktop   : ${CYAN}$DESKTOP_DIR/org.tuxixi.Terminal.desktop${RESET}"
echo ""

if [ "${PATH_NOTE:-0}" = "1" ]; then
    echo "  ${AMBER}note:${RESET} add TUXIXI to your PATH first:"
    echo "        ${LIME}export PATH=\"$BIN_DIR:\$PATH\"${RESET}"
    echo ""
fi

if ! "$BIN_DIR/tuxixi" --version 2>/dev/null; then
    echo "  ${AMBER}note:${RESET} the binary could not run yet. TUXIXI needs"
    echo "        GTK4 and VTE 3.91 at runtime, e.g.:"
    echo "        ${DIM}Debian/Ubuntu: sudo apt install libgtk-4-1 libvte-2.91-gtk4-1${RESET}"
    echo "        ${DIM}Arch: sudo pacman -S gtk4 vte4   Fedora: sudo dnf install gtk4 vte291-gtk4${RESET}"
    echo ""
fi

echo "  Launch TUXIXI from your application menu, or run"
echo "  ${LIME}tuxixi${RESET} from a terminal. TUXIXI commands"
echo "  (${LIME}tuxixi --help${RESET}) are available only inside TUXIXI Terminal."
echo ""
echo "  ${BOLD}${PURPLE}Thank you for choosing TUXIXI! <3${RESET}"
echo ""
