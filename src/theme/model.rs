// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Theme data model: [`Theme`], [`Rgba`] color parsing and the
//! [`Registry`] that merges embedded themes with user theme files.
//! User files override built-ins with the same display name, and the
//! user themes directory is scanned recursively so a cloned theme
//! repository can be dropped in as-is.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::theme_formats;

use super::data::{BUILTIN_FILES, DEFAULT_THEME};

pub const MAX_SCAN_DEPTH: usize = 4;

/// One color theme: background / foreground / cursor as `#RRGGBB`,
/// plus an optional 16-color ANSI palette (same order VTE expects:
/// 0-7 normal black..white, 8-15 bright black..white).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    #[serde(default)]
    pub cursor: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette: Vec<String>,
}

fn validate_hex_field(field: &str, value: &str, theme: &str) -> Result<(), String> {
    if Rgba::parse_hex(value).is_none() {
        return Err(format!(
            "theme '{theme}': {field} is not a valid #RRGGBB color: '{value}'"
        ));
    }
    Ok(())
}

impl Theme {
    /// Parse and validate one theme JSON string.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut theme: Theme =
            serde_json::from_str(json).map_err(|e| format!("invalid theme JSON: {e}"))?;
        theme.name = theme.name.trim().to_string();
        if theme.name.is_empty() {
            return Err("theme name is empty".to_string());
        }
        if theme.cursor.trim().is_empty() {
            theme.cursor = theme.foreground.clone();
        }
        if theme.palette.len() > 16 {
            return Err(format!(
                "theme '{}': palette has {} entries, at most 16 are supported",
                theme.name,
                theme.palette.len()
            ));
        }
        validate_hex_field("background", &theme.background, &theme.name)?;
        validate_hex_field("foreground", &theme.foreground, &theme.name)?;
        validate_hex_field("cursor", &theme.cursor, &theme.name)?;
        for (i, color) in theme.palette.iter().enumerate() {
            validate_hex_field(&format!("palette[{i}]"), color, &theme.name)?;
        }
        Ok(theme)
    }
}

/// Simple RGBA color value (0.0-1.0 components), the port of `Gdk.RGBA`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// Parse a `#RRGGBB` hex string into an opaque RGBA (alpha = 1.0).
    pub fn parse_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim();
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Self {
            r: ((v >> 16) & 0xFF) as f64 / 255.0,
            g: ((v >> 8) & 0xFF) as f64 / 255.0,
            b: (v & 0xFF) as f64 / 255.0,
            a: 1.0,
        })
    }

    pub fn black_transparent() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }

    pub fn opaque(hex: &str) -> Self {
        Self::parse_hex(hex).unwrap_or_else(Self::opaque_default)
    }

    fn opaque_default() -> Self {
        Self::new(0.0, 0.0, 0.0, 1.0)
    }
}

/// All themes available in one process: the embedded built-ins plus the
/// user themes from `~/.config/tuxixi/themes/` (a user theme replaces a
/// built-in when both use the same display name).
#[derive(Debug, Clone)]
pub struct Registry {
    themes: Vec<Theme>,
}

impl Registry {
    /// The process-wide registry: built-ins + user themes. Built lazily
    /// and then shared immutably.
    pub fn global() -> &'static Registry {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let dir = user_themes_dir();
            if dir.is_dir() {
                Registry::load(Some(&dir))
            } else {
                Registry::builtin()
            }
        })
    }

    /// Only the embedded built-in themes (no filesystem access). Every
    /// embedded file is the authentic upstream file, parsed with the
    /// same parser that handles user files.
    pub fn builtin() -> Self {
        let mut themes = Vec::new();
        for (file, content) in BUILTIN_FILES {
            let theme = theme_from_embedded(file, content);
            themes.push(theme);
        }
        Self { themes }
    }

    /// Built-ins + user themes from `dir` (scanned recursively). Invalid
    /// files are skipped with a warning on stderr; valid files override
    /// built-ins with the same name.
    pub fn load(user_dir: Option<&Path>) -> Self {
        let mut reg = Self::builtin();
        let Some(dir) = user_dir else { return reg };
        if !dir.is_dir() {
            return reg;
        }
        let mut seen_user_names: std::collections::HashSet<String> = Default::default();
        for path in collect_theme_files(dir, 0) {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Ok(content) = std::fs::read_to_string(&path) else {
                eprintln!("Warning: could not read theme file {file_name}");
                continue;
            };
            match theme_formats::theme_from_file(&path, &content) {
                Ok(theme) => {
                    // First user file with a given name wins; later ones
                    // (and built-in overrides) are ignored for that name.
                    if seen_user_names.contains(&theme.name) {
                        continue;
                    }
                    seen_user_names.insert(theme.name.clone());
                    reg.replace_or_push(theme);
                }
                Err(e) => eprintln!("Warning: skipping theme {file_name}: {e}"),
            }
        }
        reg
    }

    fn replace_or_push(&mut self, theme: Theme) {
        match self.themes.iter_mut().find(|t| t.name == theme.name) {
            Some(existing) => *existing = theme,
            None => self.themes.push(theme),
        }
    }

    /// Display names in cycling order.
    pub fn names(&self) -> Vec<String> {
        self.themes.iter().map(|t| t.name.clone()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.themes.iter().find(|t| t.name == name)
    }

    /// The theme used for unknown names: the default theme when valid,
    /// otherwise the first built-in.
    pub fn fallback(&self) -> &Theme {
        self.get(DEFAULT_THEME).unwrap_or_else(|| &self.themes[0])
    }

    pub fn len(&self) -> usize {
        self.themes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }
}

/// Parse one embedded built-in file. Panics on corruption: a broken
/// embedded theme is a programming error caught by the test suite.
fn theme_from_embedded(file: &str, content: &str) -> Theme {
    let path = Path::new(file);
    theme_formats::theme_from_file(path, content)
        .unwrap_or_else(|e| panic!("corrupt embedded theme {file}: {e}"))
}

/// Recursively collect files with a supported theme extension, sorted
/// for deterministic override order. Hidden entries and `.git` are
/// skipped; the depth is capped at [`MAX_SCAN_DEPTH`].
fn collect_theme_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth > MAX_SCAN_DEPTH {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            out.extend(collect_theme_files(&path, depth + 1));
        } else if theme_formats::SUPPORTED_EXTENSIONS
            .iter()
            .any(|ext| name.to_ascii_lowercase().ends_with(&format!(".{ext}")))
        {
            out.push(path);
        }
    }
    out
}

/// The user themes directory: `$XDG_CONFIG_HOME/tuxixi/themes` (or
/// `~/.config/tuxixi/themes`).
pub fn user_themes_dir() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            Path::new(&home).join(".config")
        });
    config_dir.join("tuxixi").join("themes")
}
