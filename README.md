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
window with tabs, 32 themes and optional transparency (glass/blur).

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

TUXIXI ships with **32 built-in themes — the real upstream theme files**,
fetched straight from their creators (via the MIT-licensed
[iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes)
collection): TokyoNight (the default), Dracula, Nord, Gruvbox Dark/Light,
Catppuccin (all four flavors), Solarized, Rosé Pine, Kanagawa, Everforest,
and more. Because the original files are used as-is, each theme carries
its complete 16-color ANSI palette, not just a background/foreground
pair.

Change the theme with `Ctrl+Shift+T`, or from the command line:

```bash
tuxixi theme list         # show all themes (current one marked)
tuxixi theme set Dracula  # switch and persist
tuxixi theme next         # cycle
```

### Your own themes — any format

Drop theme files into `~/.config/tuxixi/themes/` — the directory is
scanned recursively, so you can `git clone` a whole theme repository
into it and TUXIXI picks the files up on the next start. Supported
formats, parsed directly without any conversion:

| Format | Files | Source example |
| --- | --- | --- |
| iTerm2 | `.itermcolors` | `git clone https://github.com/mbadolato/iTerm2-Color-Schemes ~/.config/tuxixi/themes/` |
| Alacritty | `.toml`, `.yml` | `git clone https://github.com/alacritty/alacritty-theme ~/.config/tuxixi/themes/` |
| kitty | `.conf` | any kitty theme collection |
| TUXIXI native | `.json` | see below |

```json
{
  "name": "My Theme",
  "background": "#1A1B26",
  "foreground": "#A9B1D6",
  "cursor": "#A9B1D6",
  "palette": ["#000000", "... up to 16 colors ..."]
}
```

Only `cursor` is optional (defaults to the foreground); `palette` is
optional too. A user file whose display name matches a built-in
overrides it. Old hand-made theme copies that were once installed are
detected by hash and cleaned up automatically by `tuxixi update --theme`
— files you created or edited yourself are never touched.

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
