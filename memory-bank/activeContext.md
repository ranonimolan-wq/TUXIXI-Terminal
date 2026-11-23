# Active Context — TUXIXI Terminal (2026-11-23, late session)

## Session outcome
- Ghost-input report resolved: emulation core swapped to `alacritty_terminal`
  (CPR/DA responses, TERM pinning, focus-safe modifiers, no synthetic keys);
  verified on Xvfb with real zsh 5.9 + syntax-highlighting: display matches the
  shell buffer byte-for-byte; DSR probe answers `\x1b[6;1R`.
- Font rasterization switched to FreeType (hinted) for Alacritty-parity text
  crispness; CI/release workflows install libfreetype6-dev.
- Verification gates: fmt 100%, clippy --all-features 0 warnings,
  288 lib + 12 integration tests pass; no file exceeds the 800-line cap.

## Rules in force
AI_GUIDELINES.md (aethelisdev): safe Rust only, `///` docs >= 8/10, English
source + Turkish user communication, no `#[allow]`, SRP/800-line cap, no
mod.rs, knowledge + memory-bank sync per change, verification block per
change, commit format `feat(scope): ...`.

## Known follow-ups
- If the user still reports ghost input, ask for a `TUXIXI_DEBUG_INPUT=1`
  log: it now records every key event, every PTY write and every engine
  response verbatim.
- `cli.rs` (759) and `theme_formats.rs` (680) are closest to the 800-line cap.
- Copy verification on Wayland still indirect (arboard), watch reports.
