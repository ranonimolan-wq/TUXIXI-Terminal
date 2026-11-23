//! Theme system: the built-in themes are the REAL upstream theme files
//! (published by their original creators) in `themes/upstream/`,
//! embedded into the binary at compile time and parsed by
//! [`crate::theme_formats`]. Users can drop their own theme files in
//! ANY supported format (`.itermcolors`, Alacritty `.toml`/`.yml`,
//! kitty `.conf`, or native `.json`) into `~/.config/tuxixi/themes/` -
//! the directory is scanned recursively, so a whole cloned theme
//! repository works too. A user file overrides a built-in with the same
//! display name. No GTK dependency: colors are plain RGBA values.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::theme_formats;

pub const DEFAULT_THEME: &str = "TokyoNight";

/// The built-in theme files embedded into the binary (manifest-relative
/// path, content). The manifest-relative path is also the key used by
/// `tuxixi update --theme`. The order defines the theme cycling
/// sequence (Ctrl+Shift+T).
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("upstream/Atom-One-Dark.itermcolors", include_str!("../themes/upstream/Atom-One-Dark.itermcolors")),
    ("upstream/Atom-One-Light.itermcolors", include_str!("../themes/upstream/Atom-One-Light.itermcolors")),
    ("upstream/Ayu-Mirage.itermcolors", include_str!("../themes/upstream/Ayu-Mirage.itermcolors")),
    ("upstream/Ayu.itermcolors", include_str!("../themes/upstream/Ayu.itermcolors")),
    ("upstream/Catppuccin-Frappe.itermcolors", include_str!("../themes/upstream/Catppuccin-Frappe.itermcolors")),
    ("upstream/Catppuccin-Latte.itermcolors", include_str!("../themes/upstream/Catppuccin-Latte.itermcolors")),
    ("upstream/Catppuccin-Macchiato.itermcolors", include_str!("../themes/upstream/Catppuccin-Macchiato.itermcolors")),
    ("upstream/Catppuccin-Mocha.itermcolors", include_str!("../themes/upstream/Catppuccin-Mocha.itermcolors")),
    ("upstream/Cyberpunk.itermcolors", include_str!("../themes/upstream/Cyberpunk.itermcolors")),
    ("upstream/Dracula.itermcolors", include_str!("../themes/upstream/Dracula.itermcolors")),
    ("upstream/Everforest-Dark-Med.itermcolors", include_str!("../themes/upstream/Everforest-Dark-Med.itermcolors")),
    ("upstream/Flexoki-Dark.itermcolors", include_str!("../themes/upstream/Flexoki-Dark.itermcolors")),
    ("upstream/Flexoki-Light.itermcolors", include_str!("../themes/upstream/Flexoki-Light.itermcolors")),
    ("upstream/GitHub-Dark.itermcolors", include_str!("../themes/upstream/GitHub-Dark.itermcolors")),
    ("upstream/Gruvbox-Dark.itermcolors", include_str!("../themes/upstream/Gruvbox-Dark.itermcolors")),
    ("upstream/Gruvbox-Light.itermcolors", include_str!("../themes/upstream/Gruvbox-Light.itermcolors")),
    ("upstream/Kanagawa-Wave.itermcolors", include_str!("../themes/upstream/Kanagawa-Wave.itermcolors")),
    ("upstream/Monokai-Classic.itermcolors", include_str!("../themes/upstream/Monokai-Classic.itermcolors")),
    ("upstream/Night-Owl.itermcolors", include_str!("../themes/upstream/Night-Owl.itermcolors")),
    ("upstream/Nord.itermcolors", include_str!("../themes/upstream/Nord.itermcolors")),
    ("upstream/One-Half-Dark.itermcolors", include_str!("../themes/upstream/One-Half-Dark.itermcolors")),
    ("upstream/One-Half-Light.itermcolors", include_str!("../themes/upstream/One-Half-Light.itermcolors")),
    ("upstream/Rose-Pine-Dawn.itermcolors", include_str!("../themes/upstream/Rose-Pine-Dawn.itermcolors")),
    ("upstream/Rose-Pine-Moon.itermcolors", include_str!("../themes/upstream/Rose-Pine-Moon.itermcolors")),
    ("upstream/Rose-Pine.itermcolors", include_str!("../themes/upstream/Rose-Pine.itermcolors")),
    ("upstream/Synthwave.itermcolors", include_str!("../themes/upstream/Synthwave.itermcolors")),
    ("upstream/TokyoNight-Day.itermcolors", include_str!("../themes/upstream/TokyoNight-Day.itermcolors")),
    ("upstream/TokyoNight-Moon.itermcolors", include_str!("../themes/upstream/TokyoNight-Moon.itermcolors")),
    ("upstream/TokyoNight-Storm.itermcolors", include_str!("../themes/upstream/TokyoNight-Storm.itermcolors")),
    ("upstream/TokyoNight.itermcolors", include_str!("../themes/upstream/TokyoNight.itermcolors")),
    ("upstream/iTerm2-Solarized-Dark.itermcolors", include_str!("../themes/upstream/iTerm2-Solarized-Dark.itermcolors")),
    ("upstream/iTerm2-Solarized-Light.itermcolors", include_str!("../themes/upstream/iTerm2-Solarized-Light.itermcolors")),
];

/// The embedded copy of `themes/manifest.json`: SHA-256 of every built-in
/// theme file, used by `tuxixi update --theme` (hash based, no versions).
pub const MANIFEST_JSON: &str = include_str!("../themes/manifest.json");

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
    "#000000", "#CD0000", "#00CD00", "#CDCD00",
    "#0000EE", "#CD00CD", "#00CDCD", "#E5E5E5",
    "#7F7F7F", "#FF0000", "#00FF00", "#FFFF00",
    "#5C5CFF", "#FF00FF", "#00FFFF", "#FFFFFF",
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

/// How deep the user themes directory is scanned (so a whole cloned
/// theme repository works; `~/.config/tuxixi/themes/<repo>/<file>`).
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
        let mut theme: Theme = serde_json::from_str(json)
            .map_err(|e| format!("invalid theme JSON: {e}"))?;
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
        self.get(DEFAULT_THEME)
            .unwrap_or_else(|| &self.themes[0])
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
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
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
        } else if theme_formats::SUPPORTED_EXTENSIONS.iter().any(|ext| {
            name.to_ascii_lowercase().ends_with(&format!(".{ext}"))
        }) {
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
        let fg = Rgba::parse_hex(&theme.foreground)
            .unwrap_or_else(|| Rgba::opaque(GLASS_FALLBACK_FG));
        (Rgba::black_transparent(), fg)
    } else {
        (Rgba::opaque(&theme.background), Rgba::opaque(&theme.foreground))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_32_unique_valid_themes() {
        let reg = Registry::builtin();
        assert_eq!(reg.len(), 32, "32 vendored upstream themes expected");
        let names = reg.names();
        for name in &names {
            assert!(reg.get(name).is_some());
        }
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "duplicate theme names");
    }

    #[test]
    fn every_builtin_has_a_full_16_color_palette() {
        let reg = Registry::builtin();
        for name in reg.names() {
            let t = reg.get(&name).unwrap();
            assert_eq!(t.palette.len(), 16, "{name} palette incomplete");
        }
    }

    #[test]
    fn manifest_covers_every_builtin_file_with_matching_hash() {
        let manifest: serde_json::Value = serde_json::from_str(MANIFEST_JSON).unwrap();
        let entries = manifest["themes"].as_object().expect("manifest.themes");
        assert_eq!(entries.len(), BUILTIN_FILES.len());

        for (file, json) in BUILTIN_FILES {
            let recorded = entries
                .get(*file)
                .unwrap_or_else(|| panic!("manifest is missing {file}"))
                .as_str()
                .unwrap();
            let actual = crate::update::sha256_hex(json.as_bytes());
            assert_eq!(recorded, actual, "manifest hash mismatch for {file}");
        }
    }

    #[test]
    fn manifest_has_no_orphan_entries() {
        let manifest: serde_json::Value = serde_json::from_str(MANIFEST_JSON).unwrap();
        let entries = manifest["themes"].as_object().expect("manifest.themes");
        for file in entries.keys() {
            assert!(
                BUILTIN_FILES.iter().any(|(f, _)| f == file),
                "manifest lists {file} but there is no such embedded theme"
            );
        }
    }

    #[test]
    fn tokyo_night_is_the_default_and_comes_from_the_upstream_file() {
        assert_eq!(DEFAULT_THEME, "TokyoNight");
        let reg = Registry::builtin();
        let theme = reg.get(DEFAULT_THEME).expect("TokyoNight must exist");
        assert_eq!(theme.background, "#1A1B26");
        assert_eq!(theme.foreground, "#C0CAF5");
        assert_eq!(theme.cursor, "#C0CAF5");
        assert_eq!(theme.palette[1], "#F7768E");
    }

    #[test]
    fn unknown_theme_falls_back_to_tokyo_night() {
        let reg = Registry::builtin();
        assert_eq!(reg.fallback().name, "TokyoNight");
        let (bg, fg) = theme_colors("Nope");
        assert_eq!(bg, "#1A1B26");
        assert_eq!(fg, "#C0CAF5");
    }

    #[test]
    fn theme_json_parse_accepts_missing_cursor_and_palette() {
        let theme = Theme::from_json(
            r##"{"name": "X", "background": "#000000", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();
        assert_eq!(theme.cursor, "#FFFFFF");
        assert!(theme.palette.is_empty());
    }

    #[test]
    fn theme_json_parse_accepts_a_valid_palette() {
        let theme = Theme::from_json(
            r##"{"name": "P", "background": "#000000", "foreground": "#FFFFFF",
                 "palette": ["#111111","#222222","#333333","#444444","#555555","#666666","#777777","#888888",
                              "#999999","#AAAAAA","#BBBBBB","#CCCCCC","#DDDDDD","#EEEEEE","#FFFFFF","#0F0F0F"]}"##,
        )
        .unwrap();
        assert_eq!(theme.palette.len(), 16);
        assert_eq!(theme.palette[15], "#0F0F0F");
    }

    #[test]
    fn theme_json_parse_rejects_garbage() {
        assert!(Theme::from_json("not json").is_err());
        assert!(Theme::from_json(r##"{"background": "#000000"}"##).is_err()); // no name
        assert!(Theme::from_json(
            r##"{"name": "Bad", "background": "#12345", "foreground": "#FFFFFF"}"##
        )
        .is_err());
        assert!(Theme::from_json(
            r##"{"name": "  ", "background": "#000000", "foreground": "#FFFFFF"}"##
        )
        .is_err());
        assert!(Theme::from_json(
            r##"{"name": "BadCursor", "background": "#000000", "foreground": "#FFFFFF", "cursor": "#ZZZZZZ"}"##
        )
        .is_err());
        assert!(Theme::from_json(
            r##"{"name": "BadPalette", "background": "#000000", "foreground": "#FFFFFF", "palette": ["#0000GG"]}"##
        )
        .is_err());
        assert!(Theme::from_json(
            r##"{"name": "LongPalette", "background": "#000000", "foreground": "#FFFFFF",
                 "palette": ["#000000","#000000","#000000","#000000","#000000","#000000","#000000","#000000",
                              "#000000","#000000","#000000","#000000","#000000","#000000","#000000","#000000","#000000"]}"##
        )
        .is_err());
    }

    fn temp_theme_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuxixi-themes-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn user_theme_overrides_builtin_of_the_same_name() {
        let dir = temp_theme_dir("override");
        // Same file stem as the built-in -> same display name -> override.
        std::fs::write(
            dir.join("Dracula.json"),
            r##"{"name": "Dracula", "background": "#123456", "foreground": "#654321"}"##,
        )
        .unwrap();

        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 32, "override must not add a new slot");
        let dracula = reg.get("Dracula").unwrap();
        assert_eq!(dracula.background, "#123456");
        assert_eq!(dracula.foreground, "#654321");
        assert!(dracula.palette.is_empty(), "the user file has no palette");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn user_themes_are_found_recursively_and_appended() {
        let dir = temp_theme_dir("recursive");
        let repo = dir.join("some-theme-repo/themes");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join("my-theme.toml"),
            "[colors.primary]\nbackground = \"#010203\"\nforeground = \"#040506\"\n",
        )
        .unwrap();
        // .git directories must be skipped, not parsed.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/hooks.toml"), "junk").unwrap();

        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 33, "one appended user theme");
        let mine = reg.get("My Theme").unwrap();
        assert_eq!(mine.background, "#010203");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn first_user_file_wins_on_duplicate_names() {
        let dir = temp_theme_dir("dup");
        // Both files share the stem "Dupe" (the display name source), so
        // the alphabetically first one must win.
        std::fs::write(
            dir.join("Dupe.json"),
            r##"{"name": "Dupe", "background": "#000001", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();
        let sub = dir.join("z-sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join("Dupe.json"),
            r##"{"name": "Dupe", "background": "#000002", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();
        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 33);
        assert_eq!(reg.get("Dupe").unwrap().background, "#000001");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn invalid_user_theme_files_are_skipped() {
        let dir = temp_theme_dir("invalid");
        std::fs::write(dir.join("broken.json"), "{{{").unwrap();
        std::fs::write(
            dir.join("badcolor.json"),
            r##"{"name": "BadColor", "background": "red", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();
        std::fs::write(dir.join("junk.txt"), "not a theme").unwrap();
        std::fs::write(
            dir.join("good.json"),
            r##"{"name": "Good", "background": "#000000", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();

        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 33);
        assert!(reg.get("Good").is_some());
        assert!(reg.get("BadColor").is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_user_dir_means_builtins_only() {
        let reg = Registry::load(Some(Path::new("/nonexistent/tuxixi-themes-xyz")));
        assert_eq!(reg.len(), 32);
    }

    #[test]
    fn theme_colors_returns_builtin_colors() {
        let (bg, fg) = theme_colors("Dracula");
        assert_eq!(bg, "#282A36");
        assert_eq!(fg, "#F8F8F2");
        let (bg, fg) = theme_colors(DEFAULT_THEME);
        assert_eq!(bg, "#1A1B26");
        assert_eq!(fg, "#C0CAF5");
    }

    #[test]
    fn get_colors_normal_mode_uses_theme() {
        let (bg, fg) = get_colors("Dracula", false, "off");
        assert_eq!(bg, Rgba::parse_hex("#282A36").unwrap());
        assert_eq!(fg, Rgba::parse_hex("#F8F8F2").unwrap());
        assert_eq!(bg.a, 1.0);
        assert_eq!(fg.a, 1.0);
    }

    #[test]
    fn get_colors_glass_mode_keeps_the_theme_foreground() {
        for transparent in [false, true] {
            for mode in ["static", "smoky"] {
                let (bg, fg) = get_colors("Dracula", transparent, mode);
                assert_eq!(bg, Rgba::new(0.0, 0.0, 0.0, 0.0));
                assert_eq!(fg, Rgba::parse_hex("#F8F8F2").unwrap());
            }
        }
    }

    #[test]
    fn is_glass_matches_python_rule() {
        assert!(!is_glass(false, "off"));
        assert!(is_glass(true, "off"));
        assert!(is_glass(false, "static"));
        assert!(is_glass(false, "smoky"));
    }

    #[test]
    fn parse_hex_rejects_garbage() {
        assert!(Rgba::parse_hex("").is_none());
        assert!(Rgba::parse_hex("#12345").is_none());
        assert!(Rgba::parse_hex("#12345G").is_none());
        assert!(Rgba::parse_hex("123456").is_some()); // no # but 6 digits is ok
    }

    #[test]
    fn constants_are_stable() {
        assert_eq!(BLUR_MODES, &["off", "static", "smoky"]);
        assert!((TRANSLUCENT_ALPHA - 0.52).abs() < f64::EPSILON);
        assert!((SMOKY_ALPHA - 0.87).abs() < f64::EPSILON);
    }

    #[test]
    fn terminal_colors_resolve_every_builtin_theme_completely() {
        let reg = Registry::builtin();
        for name in reg.names() {
            let t = reg.get(&name).unwrap();
            let c = terminal_colors(&name, false, "off");
            assert_eq!(c.background, Rgba::opaque(&t.background), "{name} bg");
            assert_eq!(c.foreground, Rgba::opaque(&t.foreground), "{name} fg");
            assert_eq!(c.cursor, Rgba::opaque(&t.cursor), "{name} cursor");
            assert_eq!(
                c.palette.len(),
                t.palette.len(),
                "{name} palette length must survive resolution"
            );
            for (resolved, hex) in c.palette.iter().zip(t.palette.iter()) {
                assert_eq!(resolved, &Rgba::opaque(hex), "{name} palette entry {hex}");
            }
        }
    }

    #[test]
    fn terminal_colors_unknown_theme_falls_back_to_tokyo_night() {
        let c = terminal_colors("This-Theme-Does-Not-Exist", false, "off");
        assert_eq!(c.background, Rgba::parse_hex("#1A1B26").unwrap());
        assert_eq!(c.foreground, Rgba::parse_hex("#C0CAF5").unwrap());
    }

    #[test]
    fn terminal_colors_normal_mode_is_opaque() {
        let c = terminal_colors("Dracula", false, "off");
        assert_eq!(c.background.a, 1.0);
        assert_eq!(c.foreground.a, 1.0);
        assert_eq!(c.cursor.a, 1.0);
        for entry in &c.palette {
            assert_eq!(entry.a, 1.0, "palette entries must be opaque");
        }
    }

    #[test]
    fn terminal_colors_glass_mode_is_transparent_but_keeps_theme_fg() {
        for transparent in [false, true] {
            for mode in ["static", "smoky"] {
                let c = terminal_colors("Dracula", transparent, mode);
                assert_eq!(c.background, Rgba::new(0.0, 0.0, 0.0, 0.0));
                assert_eq!(c.foreground, Rgba::parse_hex("#F8F8F2").unwrap());
            }
        }
    }

    #[test]
    fn terminal_colors_differ_between_themes_so_switching_is_visible() {
        let a = terminal_colors("Dracula", false, "off");
        let b = terminal_colors("Nord", false, "off");
        assert_ne!(a.background, b.background, "themes must paint different backgrounds");
        assert_eq!(a.palette.len(), b.palette.len());
    }

    // ---- regression tests: palettless themes must never crash ----

    fn bare_theme(name: &str) -> Theme {
        Theme {
            name: name.to_string(),
            background: "#1A1B26".to_string(),
            foreground: "#C0CAF5".to_string(),
            cursor: "#C0CAF5".to_string(),
            palette: Vec::new(),
        }
    }

    #[test]
    fn resolve_palette_gives_sixteen_slots_for_a_paletteless_theme() {
        // The reported startup crash: a user theme JSON with only
        // background/foreground (no palette) reached the renderer as a
        // zero-length palette. Resolution must always produce 16 slots.
        let resolved = resolve_palette(&bare_theme("Bare"));
        assert_eq!(resolved.len(), 16);
        for (i, entry) in resolved.iter().enumerate() {
            assert_eq!(*entry, standard_ansi(i), "slot {i} must be the standard base");
            assert_eq!(entry.a, 1.0, "slot {i} must be opaque");
        }
    }

    #[test]
    fn resolve_palette_overlays_a_partial_theme_on_the_standard_base() {
        let mut t = bare_theme("Partial");
        t.palette = vec!["#ABCDEF".to_string(), "#112233".to_string()];
        let resolved = resolve_palette(&t);
        assert_eq!(resolved.len(), 16);
        assert_eq!(resolved[0], Rgba::opaque("#ABCDEF"));
        assert_eq!(resolved[1], Rgba::opaque("#112233"));
        assert_eq!(resolved[2], standard_ansi(2), "missing slot 2 keeps the base");
        assert_eq!(resolved[15], standard_ansi(15), "missing slot 15 keeps the base");
    }

    #[test]
    fn resolve_palette_skips_unparsable_entries() {
        let mut t = bare_theme("Garbage");
        t.palette = vec!["#GGGGGG".to_string(), "#0000FF".to_string()];
        let resolved = resolve_palette(&t);
        assert_eq!(resolved.len(), 16);
        assert_eq!(resolved[0], standard_ansi(0), "garbage entry keeps the base");
        assert_eq!(resolved[1], Rgba::opaque("#0000FF"));
    }

    #[test]
    fn resolve_palette_ignores_extra_entries_beyond_sixteen() {
        // Theme::from_json rejects >16, but resolve_palette must stay
        // safe even if a theme object is built programmatically.
        let mut t = bare_theme("TooMany");
        t.palette = vec!["#ABCDEF".to_string(); 20];
        let resolved = resolve_palette(&t);
        assert_eq!(resolved.len(), 16);
    }

    #[test]
    fn terminal_colors_always_yields_sixteen_palette_entries() {
        let reg = Registry::builtin();
        for name in reg.names() {
            let c = terminal_colors(&name, false, "off");
            assert_eq!(c.palette.len(), 16, "{name}");
        }
    }

    #[test]
    fn palettless_user_override_survives_the_full_color_pipeline() {
        // The exact user scenario: a palettless Dracula.json in the
        // user themes dir overrides the built-in, the app starts with
        // that theme selected, and the renderer palette is built from
        // it. End-to-end, this must produce a usable 16-slot palette.
        let dir = temp_theme_dir("palettless");
        std::fs::write(
            dir.join("Dracula.json"),
            r##"{"name": "Dracula", "background": "#123456", "foreground": "#654321"}"##,
        )
        .unwrap();
        let reg = Registry::load(Some(&dir));
        let theme = reg.get("Dracula").unwrap();
        assert!(theme.palette.is_empty(), "fixture sanity: no palette in the file");

        let resolved = resolve_palette(theme);
        assert_eq!(resolved.len(), 16);

        let colors = TerminalColors {
            background: Rgba::opaque(&theme.background),
            foreground: Rgba::opaque(&theme.foreground),
            cursor: Rgba::opaque(&theme.cursor),
            palette: resolved,
        };
        // The exact call that used to panic (render.rs:30).
        #[cfg(feature = "gui")]
        {
            let palette = crate::render::Palette::from_terminal_colors(&colors);
            assert_eq!(palette.ansi.len(), 16);
            assert_eq!(palette.background, [0x12, 0x34, 0x56]);
            assert_eq!(palette.foreground, [0x65, 0x43, 0x21]);
            assert_eq!(palette.ansi[1], [0xCD, 0x00, 0x00], "standard red fallback");
        }
        let _ = colors; // silence unused warning in CLI-only test builds
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn standard_ansi_table_is_valid_and_distinct() {
        assert_eq!(STANDARD_ANSI_16.len(), 16);
        for (i, hex) in STANDARD_ANSI_16.iter().enumerate() {
            let c = Rgba::parse_hex(hex)
                .unwrap_or_else(|| panic!("STANDARD_ANSI_16[{i}] = {hex} is not a valid hex"));
            assert_eq!(c.a, 1.0);
        }
        let unique: std::collections::HashSet<_> = STANDARD_ANSI_16.iter().collect();
        assert_eq!(unique.len(), 16, "standard ANSI colors must be distinct");
        // Out-of-range index clamps instead of panicking.
        assert_eq!(standard_ansi(16), standard_ansi(15));
        assert_eq!(standard_ansi(9999), standard_ansi(15));
    }
}
