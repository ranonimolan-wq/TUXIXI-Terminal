# Upstream themes (creator originals)

The `.itermcolors` files in this directory are NOT hand-written by the
TUXIXI project. They are the original theme files published by their
creators, fetched from:

    https://github.com/mbadolato/iTerm2-Color-Schemes

That repository collects the authentic color schemes (Tokyo Night by
 enkia/folke, Dracula by Zeno Rocha, Catppuccin by the Catppuccin team,
Gruvbox by morhetz, Nord by arcticicestudio, Solarized by Ethan
Schoonover, and so on) in iTerm2's plist format, and is MIT licensed.
Spaces in the original scheme names are replaced by `-` in file names
(`TokyoNight Storm` -> `TokyoNight-Storm.itermcolors`); the display
name inside TUXIXI is derived back from the file name.

TUXIXI reads these files directly (no conversion step), which also
means any user can `git clone` that repository - or an Alacritty
(`.toml`/`.yml`), kitty (`.conf`) or JSON theme collection - into
`~/.config/tuxixi/themes/` and TUXIXI will pick the files up on the
next start. See `src/theme_formats.rs` for the supported formats.

To refresh this directory run:

    sh scripts/fetch_upstream_themes.sh
    python3 scripts/gen_theme_manifest.py
