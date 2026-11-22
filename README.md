<div align="center">

# TUXIXI Terminal

A small terminal emulator for Linux, written in Rust.

![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange) ![GTK4 + VTE](https://img.shields.io/badge/GTK4-VTE%203.91-blue) ![Platform](https://img.shields.io/badge/Platform-Linux-green) ![License](https://img.shields.io/badge/License-GPL--3.0-yellow)

</div>

This is an independent fork of TUXIXI-Terminal, rebuilt on a Rust core instead
of Python. The old Python code is gone; everything now lives in a single Cargo
project at the repository root. Releases are built automatically by GitHub
Actions.

The GUI is GTK4 + VTE, so you get a real terminal — your own shell (Bash, Zsh,
Fish), scrollback, copy/paste, TrueColor — wrapped in a small, customizable
window with tabs, 19 themes and optional transparency (glass/blur).

## Install

Install the latest release with a single command — the installer downloads
the binary and sets it up in `~/.local` for you:

```bash
curl -fsSL https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main/install.sh | bash
```

No root required. The binary needs GTK4 and VTE 3.91 at runtime
(`libgtk-4-1` and `libvte-2.91-gtk4-1` on Debian/Ubuntu, `vte4` on Arch,
`vte291-gtk4` on Fedora — most desktop systems already have them).

Prefer to review the script before running it?

```bash
curl -fsSL -o install.sh https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main/install.sh
less install.sh
chmod +x install.sh && ./install.sh
```

To uninstall, delete `~/.local/bin/tuxixi`, the
`~/.local/share/applications/org.tuxixi.Terminal.desktop` file and
`~/.local/share/icons/hicolor/512x512/apps/tuxixi-512.png`.

## CLI

```bash
tuxixi --help        # commands and shortcuts
tuxixi --version     # version information
tuxixi --sys         # kernel, uptime, load, RAM, disk
tuxixi --ip          # interface, internal and external IP
```

## License

GPL-3.0. See [LICENSE](LICENSE).
