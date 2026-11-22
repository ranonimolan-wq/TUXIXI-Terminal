#!/bin/sh
# Fetches the real, upstream theme files from the iTerm2-Color-Schemes
# repository (mbadolato/iTerm2-Color-Schemes, MIT licensed; each scheme
# was authored by its original theme creator). Files are stored under
# themes/upstream/ with spaces replaced by '-' and a flat ASCII slug.
set -e

BASE="https://raw.githubusercontent.com/mbadolato/iTerm2-Color-Schemes/master/schemes"
DEST="$(dirname "$0")/../themes/upstream"
mkdir -p "$DEST"

THEMES="
TokyoNight
TokyoNight Day
TokyoNight Storm
TokyoNight Moon
Dracula
Gruvbox Dark
Gruvbox Light
Nord
Catppuccin Mocha
Catppuccin Latte
Catppuccin Frappe
Catppuccin Macchiato
iTerm2 Solarized Dark
iTerm2 Solarized Light
Atom One Dark
Atom One Light
One Half Dark
One Half Light
Monokai Classic
Kanagawa Wave
Everforest Dark Med
Ayu
Ayu Mirage
Flexoki Dark
Flexoki Light
Night Owl
GitHub Dark
Cyberpunk
Synthwave
Rose Pine
Rose Pine Dawn
Rose Pine Moon
"

ok=0
fail=""
for name in $THEMES; do
    [ -z "$name" ] && continue
    # Re-join multi-word names that were split by $IFS.
    :
done

# The list above is space separated; process it line-wise instead.
printf '%s\n' "$THEMES" | while IFS= read -r name; do
    [ -z "$name" ] && continue
    slug=$(printf '%s' "$name" | sed 's/é/e/g; s/ /-/g')
    url="$BASE/$(printf '%s' "$name" | sed 's/ /%20/g; s/é/%C3%A9/g').itermcolors"
    if curl -fsSL "$url" -o "$DEST/$slug.itermcolors"; then
        echo "ok  $slug.itermcolors"
    else
        echo "FAIL $name ($url)"
        rm -f "$DEST/$slug.itermcolors"
    fi
done

echo "---"
ls "$DEST" | wc -l
