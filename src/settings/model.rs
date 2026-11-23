// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! `config.json` data model: validated settings and their defaults.
//!
//! [`Settings::validate_raw`] is the single normalization point: it
//! drops invalid fields back to defaults, clamps ranges and applies
//! one-time migrations, so every consumer sees a coherent snapshot.

use serde::{Deserialize, Serialize};

use super::blur_mode::BlurMode;
use crate::theme::{theme_colors, Registry, BLUR_MODES, DEFAULT_THEME};

pub const DEFAULT_FONT: &str = "Monospace";
/// Font size in pixels. 15px matches Alacritty's default (11.25pt at
/// 96dpi ≈ 15px); 11px (the old TUXIXI default) rendered noticeably
/// smaller than every other mainstream terminal.
pub const DEFAULT_FONT_SIZE: u32 = 15;
/// Configs saved by older rolling builds (before the Alacritty-parity
/// bump) store this value as `font_size`. On load it is remapped to
/// [`DEFAULT_FONT_SIZE`] so existing users actually see the fix.
/// Trade-off: 11 can no longer be *persisted*; Ctrl+Shift+Minus can
/// still reach it at runtime, but the next start migrates it up again.
pub const LEGACY_DEFAULT_FONT_SIZE: u32 = 11;
pub const MIN_FONT_SIZE: u32 = 6;
pub const MAX_FONT_SIZE: u32 = 48;

/// Glass mode defaults. Glass = translucent terminal background with a
/// compositor blur-behind request (see `crate::glass`), so the desktop
/// bleeds through the empty parts of the window while text stays crisp.
pub const DEFAULT_GLASS_OPACITY: f32 = 0.80;
pub const MIN_GLASS_OPACITY: f32 = 0.10;
pub const MAX_GLASS_OPACITY: f32 = 1.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub font: String,
    pub font_size: u32,
    pub theme: String,
    pub transparent: bool,
    pub blur_mode: BlurMode,
    /// Glass mode: translucent background + compositor blur.
    pub glass: bool,
    /// Background opacity used when `glass` is on (MIN..=MAX).
    pub glass_opacity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font: DEFAULT_FONT.to_string(),
            font_size: DEFAULT_FONT_SIZE,
            theme: DEFAULT_THEME.to_string(),
            transparent: false,
            blur_mode: BlurMode::Off,
            glass: true,
            glass_opacity: DEFAULT_GLASS_OPACITY,
        }
    }
}

impl Settings {
    /// Port of `SettingsManager._validate`: every invalid field silently
    /// falls back to its default, out-of-range font sizes are clamped,
    /// and the legacy 11px default font size migrates up to the current
    /// default (see [`LEGACY_DEFAULT_FONT_SIZE`]). The legacy
    /// `"blur": true` key (pre 2.1 config format) maps to `static` blur
    /// when `blur_mode` itself is invalid.
    pub fn validate_raw(raw: &serde_json::Value) -> Self {
        let mut s = Self::default();

        if !raw.is_object() {
            return s;
        }
        let obj = raw.as_object().unwrap();

        if let Some(v) = obj.get("font") {
            if let Some(font) = v.as_str() {
                let font = font.trim();
                if !font.is_empty() {
                    s.font = font.to_string();
                }
            }
        }

        if let Some(v) = obj.get("font_size") {
            // Python excludes bools: in JSON, booleans are not numbers
            // for serde, so `as_u64` already rejects them.
            if let Some(size) = v.as_u64() {
                s.font_size = (size as u32).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
            }
        }

        // Migration: a config written by an older rolling build stores
        // the old 11px default. Upgrade it so the bigger Alacritty-parity
        // size takes effect instead of silently keeping the tiny font.
        if s.font_size == LEGACY_DEFAULT_FONT_SIZE {
            s.font_size = DEFAULT_FONT_SIZE;
        }

        if let Some(v) = obj.get("theme") {
            if let Some(theme) = v.as_str() {
                if Registry::global().get(theme).is_some() {
                    s.theme = theme.to_string();
                }
            }
        }

        if let Some(v) = obj.get("transparent") {
            if let Some(t) = v.as_bool() {
                s.transparent = t;
            }
        }

        if let Some(v) = obj.get("blur_mode") {
            if let Some(mode) = v.as_str() {
                if let Ok(m) = mode.parse::<BlurMode>() {
                    s.blur_mode = m;
                }
            }
        }

        if let Some(v) = obj.get("glass") {
            if let Some(g) = v.as_bool() {
                s.glass = g;
            }
        }

        if let Some(v) = obj.get("glass_opacity") {
            // Numbers only, no booleans, no NaN/inf, clamped to range.
            if let Some(op) = v.as_f64().filter(|x| x.is_finite()) {
                s.glass_opacity = (op as f32).clamp(MIN_GLASS_OPACITY, MAX_GLASS_OPACITY);
            }
        }

        // Legacy compatibility: {"blur": true} meant static blur.
        if (!matches!(obj.get("blur_mode"), Some(serde_json::Value::String(_)))
            || s.blur_mode == BlurMode::Off)
            && obj.get("blur").and_then(|b| b.as_bool()) == Some(true)
        {
            // Python only applies the legacy key when blur_mode itself
            // was invalid; keep the same precedence.
            let mode_invalid = match obj.get("blur_mode") {
                Some(serde_json::Value::String(m)) => m.parse::<BlurMode>().is_err(),
                _ => true,
            };
            if mode_invalid {
                s.blur_mode = BlurMode::Static;
            }
        }

        s
    }

    pub fn theme_colors(&self) -> (String, String) {
        theme_colors(&self.theme)
    }
}

/// True when `theme` is a valid theme name (built-in or user theme).
pub fn is_valid_theme(theme: &str) -> bool {
    Registry::global().get(theme).is_some()
}

/// True when `mode` is a valid blur mode string.
pub fn is_valid_blur_mode(mode: &str) -> bool {
    BLUR_MODES.contains(&mode)
}
