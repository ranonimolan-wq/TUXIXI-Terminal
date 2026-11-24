# Engine Features — TUXIXI Terminal

| Date | Module | Feature | Notes |
|------|--------|---------|-------|
| 2026-11-23 | vt | Terminal emulation core swapped to `alacritty_terminal` 0.26 | Replaces the hand-written `vte::Perform` grid; the project-facing `Terminal` API (feed/resize/cursor/copy_visible) wraps the engine, and query responses (CPR, DA, OSC color/clipboard) surface as `TermEvent`s the GUI answers over the PTY. Scrollback bounded via `Config::scrolling_history` (10 000 rows). |
| 2026-11-23 | font | FreeType hinted rasterization | `freetype-rs` replaces the unhinted outline scaler; hinted grayscale bitmaps (LOAD_DEFAULT + TARGET_NORMAL), metrics from FreeType size metrics, integer pixel sizes. Build needs libfreetype6-dev + pkg-config; the runtime links libfreetype.so.6. |
| 2026-11-23 | shell | Pinned session TERM | Sessions now start with `TERM=xterm-256color` + `COLORTERM=truecolor` regardless of how the GUI was launched. |
| 2026-11-23 | gui | Input hardening | Tracked modifiers reset on focus loss; synthetic keyboard events dropped; permanent `TUXIXI_DEBUG_INPUT=1` trace logs key events, PTY writes and PTY responses (ground truth for input bug reports). |
| 2026-11-24 | clip | Dual-selection pasteboard layer | `src/clip.rs` wraps copypasta (X11 CLIPBOARD + PRIMARY, one long-lived server thread per selection). Drag-release copies to both selections, middle-click pastes PRIMARY, OSC 52 Clipboard/Selection requests route to the matching X11 selection. |
