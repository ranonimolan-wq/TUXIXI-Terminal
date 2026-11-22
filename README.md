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
window with tabs, 30 themes and optional transparency (glass/blur).

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

## Themes

TUXIXI ships with 30 built-in themes — Tokyo Night is the default, plus
Dracula, Nord, Gruvbox, Catppuccin, Rosé Pine, Kanagawa and more. Cycle
through them with `Ctrl+Shift+T`.

You can add your own themes: drop a JSON file into
`~/.config/tuxixi/themes/` and it is picked up on the next start (it
cycles together with the built-ins, and a file with the same name as a
built-in overrides it):

```json
{
  "name": "My Theme",
  "background": "#1A1B26",
  "foreground": "#A9B1D6",
  "cursor": "#A9B1D6"
}
```

Only `cursor` is optional — it defaults to the foreground color.

## Updates

`tuxixi update` keeps your installation fresh — no version numbers, no
manual downloads. Everything is compared by SHA-256: if the local file's
hash differs from the upstream one, the new copy is downloaded, otherwise
nothing happens.

```bash
tuxixi update            # update the binary and the themes
tuxixi update --self     # update the tuxixi binary itself
tuxixi update --theme    # sync the theme collection
tuxixi update --check    # only show what would change
```

* `--theme` syncs `themes/` from the repository into
  `~/.config/tuxixi/themes/`, so new themes are available as soon as
  they land in the repo. Your local edits are backed up as `.bak`
  before an overwrite.
* `--self` compares the running binary against the latest release and
  replaces it atomically when the hashes differ.

## License

GPL-3.0. See [LICENSE](LICENSE).
