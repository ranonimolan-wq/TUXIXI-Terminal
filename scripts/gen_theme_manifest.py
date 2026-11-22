#!/usr/bin/env python3
"""Regenerate themes/manifest.json from the theme files in themes/.

Every theme file is hashed with SHA-256. The manifest lets
`tuxixi update --theme` pick up new or changed themes without any
version numbers: it compares local file hashes against the manifest.

The manifest also carries a "retired" section: hashes of theme files
that were removed from the repository. Copies of those (exact hash
match) in the user themes directory are cleaned up automatically by
`tuxixi update --theme` -- user-created or modified files are never
touched. When regenerating, the previous "retired" entries are kept
and current theme files that disappeared from themes/upstream/ are
moved into "retired".

Usage (from the repository root):

    python3 scripts/gen_theme_manifest.py
"""

import hashlib
import json
import sys
from pathlib import Path

THEMES_DIR = Path(__file__).resolve().parent.parent / "themes"
MANIFEST = THEMES_DIR / "manifest.json"
IGNORED_FILES = {"manifest.json", "README.md"}


def main() -> int:
    if not THEMES_DIR.is_dir():
        print(f"error: {THEMES_DIR} not found", file=sys.stderr)
        return 1

    themes = {}
    # Native JSON themes (kept for user convenience) ...
    for path in sorted(THEMES_DIR.glob("*.json")):
        if path.name in IGNORED_FILES:
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        themes[path.name] = digest
    # ... and every upstream creator file (any extension).
    upstream = THEMES_DIR / "upstream"
    if upstream.is_dir():
        for path in sorted(upstream.iterdir()):
            if path.is_file() and path.name not in IGNORED_FILES:
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                themes[f"upstream/{path.name}"] = digest

    # Preserve retired entries: old hashes must keep cleaning up stale
    # user copies even after regenerations. Theme files that existed in
    # the previous manifest but vanished now become retired.
    previous_retired = {}
    previous_themes = {}
    if MANIFEST.exists():
        try:
            old = json.loads(MANIFEST.read_text())
            previous_retired = old.get("retired", {})
            previous_themes = old.get("themes", {})
        except (json.JSONDecodeError, OSError) as e:
            print(f"warning: could not read previous manifest: {e}", file=sys.stderr)
    retired = dict(previous_retired)
    for path, digest in previous_themes.items():
        if path not in themes and digest not in retired.values():
            retired[path] = digest

    manifest = {
        "schema": 1,
        "description": "SHA-256 of every built-in theme file; used by 'tuxixi update --theme'. 'retired' hashes are old built-in copies removed from user directories automatically.",
        "themes": themes,
        "retired": retired,
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {MANIFEST} ({len(themes)} themes, {len(retired)} retired)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
