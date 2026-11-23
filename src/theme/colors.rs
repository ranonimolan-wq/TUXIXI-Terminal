// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Color resolution: turn a theme name plus glass mode into the
//! concrete background / foreground / cursor / ANSI palette values a
//! terminal needs. Unknown theme names fall back to the default.

use super::model::{Registry, Rgba, Theme};

pub const BLUR_MODES: &[&str] = &["off", "static", "smoky"];

pub const TRANSLUCENT_ALPHA: f64 = 0.52;
pub const SMOKY_ALPHA: f64 = 0.87;

/// Readable fallback used in glass mode only when a theme foreground
/// cannot be parsed at all.
pub const GLASS_FALLBACK_FG: &str = "#F8FAFC";

/// Classic xterm 0-15 RGB values. They back-fill the ANSI palette of
/// themes that ship without one (or with fewer than 16 entries), so the
/// renderer always receives a full 16-slot palette.
pub const STANDARD_ANSI_16: [&str; 16] = [
    "#000000", "#CD0000", "#00CD00", "#CDCD00", "#0000EE", "#CD00CD", "#00CDCD", "#E5E5E5",
    "#7F7F7F", "#FF0000", "#00FF00", "#FFFF00", "#5C5CFF", "#FF00FF", "#00FFFF", "#FFFFFF",
];

/// The standard base color for ANSI slot `i`. Out-of-range indexes
/// clamp to white so a bad slot can never crash the renderer.
pub fn standard_ansi(i: usize) -> Rgba {
    Rgba::opaque(STANDARD_ANSI_16.get(i).copied().unwrap_or("#FFFFFF"))
}

/// A full 16-entry ANSI palette for `theme`: the standard xterm base
/// with the theme's own entries overlaid on top. A theme may legally
/// ship zero palette entries (a bare JSON file with only
/// background/foreground — including user files that override a
/// built-in of the same name); those must render with usable colors,
/// never panic. Unparsable entries also fall back to the base.
pub fn resolve_palette(theme: &Theme) -> Vec<Rgba> {
    let mut palette: Vec<Rgba> = (0..16).map(standard_ansi).collect();
    for (i, hex) in theme.palette.iter().enumerate().take(16) {
        if let Some(c) = Rgba::parse_hex(hex) {
            palette[i] = c;
        }
    }
    palette
}

/// (background, foreground) hex colors for a theme name. Unknown names
/// fall back to the default theme (TokyoNight).
pub fn theme_colors(theme_name: &str) -> (String, String) {
    let reg = Registry::global();
    let theme = reg.get(theme_name).unwrap_or_else(|| reg.fallback());
    (theme.background.clone(), theme.foreground.clone())
}

/// Port of `get_colors`: when glass mode is on (transparent, or static /
/// smoky blur) the terminal background becomes fully transparent but the
/// THEME foreground is kept, so switching themes stays visible in every
/// mode. Without glass the theme's background + foreground are used.
pub fn get_colors(theme_name: &str, transparent: bool, blur_mode: &str) -> (Rgba, Rgba) {
    let glass = transparent || blur_mode == "static" || blur_mode == "smoky";
    let reg = Registry::global();
    let theme = reg.get(theme_name).unwrap_or_else(|| reg.fallback());
    if glass {
        let fg =
            Rgba::parse_hex(&theme.foreground).unwrap_or_else(|| Rgba::opaque(GLASS_FALLBACK_FG));
        (Rgba::black_transparent(), fg)
    } else {
        (
            Rgba::opaque(&theme.background),
            Rgba::opaque(&theme.foreground),
        )
    }
}

/// Is the given blur mode / transparency combination "glass"?
pub fn is_glass(transparent: bool, blur_mode: &str) -> bool {
    transparent || blur_mode == "static" || blur_mode == "smoky"
}

/// Fully resolved colors for one VTE terminal: background, foreground,
/// cursor and the 16-color ANSI palette. The GUI converts these to
/// `gdk::RGBA` values; being plain [`Rgba`] the whole pipeline stays
/// testable without GTK.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalColors {
    pub background: Rgba,
    pub foreground: Rgba,
    pub cursor: Rgba,
    pub palette: Vec<Rgba>,
}

/// Resolve every color a terminal needs for the given theme and glass
/// mode. Unknown theme names fall back to the default theme (TokyoNight)
/// exactly like [`get_colors`]. This is the single source of truth used
/// by the GUI both at startup and whenever the user switches the theme
/// live (Ctrl+Shift+T) — the one-window-per-terminal build always opens
/// with the currently selected theme.
pub fn terminal_colors(theme_name: &str, transparent: bool, blur_mode: &str) -> TerminalColors {
    let reg = Registry::global();
    let theme = reg.get(theme_name).unwrap_or_else(|| reg.fallback());
    let (background, foreground) = get_colors(theme_name, transparent, blur_mode);
    TerminalColors {
        background,
        foreground,
        cursor: Rgba::opaque(&theme.cursor),
        palette: resolve_palette(theme),
    }
}
