# Progress — TUXIXI Terminal

## 2026-11-23 (late session)
- fix(vt): terminal engine swapped to alacritty_terminal 0.26; CPR/DA/color
  query responses answered over the PTY (previous engine never responded).
- fix(shell): session TERM pinned to xterm-256color + COLORTERM=truecolor.
- fix(gui): modifiers reset on focus loss; synthetic key events dropped.
- feat(font): FreeType hinted rasterization via freetype-rs 0.38.
- chore(ci/release): install pkg-config + libfreetype6-dev before builds.
- feat(debug): permanent TUXIXI_DEBUG_INPUT trace (key events, PTY I/O).
- docs: knowledge/history/bug_fixes.md + engine_features.md updated,
  memory bank synced. Previous session's AltGr row remains in bug_fixes.

- 2026-11-24: dual-selection clipboard (copypasta) shipped; RAM comparison documented; all checks green (clippy 0, fmt 100%, 209 tests).
