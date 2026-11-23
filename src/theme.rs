// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Theme system: the built-in themes are the REAL upstream theme files
//! (published by their original creators) in `themes/upstream/`,
//! embedded into the binary at compile time and parsed by
//! [`crate::theme_formats`]. Users can drop their own theme files in
//! ANY supported format (`.itermcolors`, Alacritty `.toml`/`.yml`,
//! kitty `.conf`, or native `.json`) into `~/.config/tuxixi/themes/` -
//! the directory is scanned recursively, so a whole cloned theme
//! repository works too. A user file overrides a built-in with the same
//! display name. No GTK dependency: colors are plain RGBA values.

mod colors;
mod data;
mod model;

pub use colors::{
    get_colors, is_glass, resolve_palette, standard_ansi, terminal_colors, theme_colors,
    TerminalColors, BLUR_MODES, GLASS_FALLBACK_FG, SMOKY_ALPHA, STANDARD_ANSI_16,
    TRANSLUCENT_ALPHA,
};
pub use data::{BUILTIN_FILES, DEFAULT_THEME, MANIFEST_JSON};
pub use model::{user_themes_dir, Registry, Rgba, Theme, MAX_SCAN_DEPTH};

#[cfg(test)]
mod tests;
