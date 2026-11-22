#!/usr/bin/env python3
"""Regenerate themes/manifest.json from the theme files in themes/.

Every theme file is hashed with SHA-256. The manifest lets
`tuxixi update --theme` pick up new or changed themes without any
version numbers: it compares local file hashes against the manifest.

Usage (from the repository root):

    python3 scripts/gen_theme_manifest.py
"""

import hashlib
import json
import sys
from pathlib import Path

THEMES_DIR = Path(__file__).resolve().parent.parent / "themes"
MANIFEST = THEMES_DIR / "manifest.json"


def main() -> int:
    if not THEMES_DIR.is_dir():
        print(f"error: {THEMES_DIR} not found", file=sys.stderr)
        return 1

    themes = {}
    for path in sorted(THEMES_DIR.glob("*.json")):
        if path.name == MANIFEST.name:
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        themes[path.name] = digest

    manifest = {
        "schema": 1,
        "description": "SHA-256 of every built-in theme file; used by 'tuxixi update --theme'.",
        "themes": themes,
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {MANIFEST} ({len(themes)} themes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
