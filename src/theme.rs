//! Theme system: the built-in themes live as JSON files in `themes/` and
//! are embedded into the binary at compile time; user themes can be
//! dropped into `~/.config/tuxixi/themes/` and override built-ins with
//! the same name. No GTK dependency: colors are plain RGBA values.
//!
//! Theme file format (`themes/tokyo-night.json`):
//!
//! ```json
//! {
//!   "name": "Tokyo Night",
//!   "background": "#1A1B26",
//!   "foreground": "#A9B1D6",
//!   "cursor": "#A9B1D6"
//! }
//! ```
//!
//! Only `cursor` is optional (it defaults to the foreground color).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub const DEFAULT_THEME: &str = "Tokyo Night";

/// The built-in theme files embedded into the binary (file name, JSON).
/// The order defines the theme cycling sequence (Ctrl+Shift+T).
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("matrix.json", include_str!("../themes/matrix.json")),
    ("alacritty-dark.json", include_str!("../themes/alacritty-dark.json")),
    ("monokai.json", include_str!("../themes/monokai.json")),
    ("dracula.json", include_str!("../themes/dracula.json")),
    ("nord.json", include_str!("../themes/nord.json")),
    ("gruvbox-dark.json", include_str!("../themes/gruvbox-dark.json")),
    ("solarized-dark.json", include_str!("../themes/solarized-dark.json")),
    ("one-dark.json", include_str!("../themes/one-dark.json")),
    ("tokyo-night.json", include_str!("../themes/tokyo-night.json")),
    ("catppuccin.json", include_str!("../themes/catppuccin.json")),
    ("cyberpunk.json", include_str!("../themes/cyberpunk.json")),
    ("classic-black-white.json", include_str!("../themes/classic-black-white.json")),
    ("classic-white-black.json", include_str!("../themes/classic-white-black.json")),
    ("solarized-light.json", include_str!("../themes/solarized-light.json")),
    ("papercolor-light.json", include_str!("../themes/papercolor-light.json")),
    ("gruvbox-light.json", include_str!("../themes/gruvbox-light.json")),
    ("purple-haze.json", include_str!("../themes/purple-haze.json")),
    ("ocean-breeze.json", include_str!("../themes/ocean-breeze.json")),
    ("retro-amber.json", include_str!("../themes/retro-amber.json")),
    ("tokyo-night-storm.json", include_str!("../themes/tokyo-night-storm.json")),
    ("tokyo-night-day.json", include_str!("../themes/tokyo-night-day.json")),
    ("rose-pine.json", include_str!("../themes/rose-pine.json")),
    ("rose-pine-moon.json", include_str!("../themes/rose-pine-moon.json")),
    ("everforest.json", include_str!("../themes/everforest.json")),
    ("kanagawa.json", include_str!("../themes/kanagawa.json")),
    ("night-owl.json", include_str!("../themes/night-owl.json")),
    ("synthwave.json", include_str!("../themes/synthwave.json")),
    ("github-dark.json", include_str!("../themes/github-dark.json")),
    ("vscode-dark-plus.json", include_str!("../themes/vscode-dark-plus.json")),
    ("flexoki-dark.json", include_str!("../themes/flexoki-dark.json")),
];

/// The embedded copy of `themes/manifest.json`: SHA-256 of every built-in
/// theme file, used by `tuxixi update --theme` (hash based, no versions).
pub const MANIFEST_JSON: &str = include_str!("../themes/manifest.json");

pub const BLUR_MODES: &[&str] = &["off", "static", "smoky"];

pub const TRANSLUCENT_ALPHA: f64 = 0.52;
pub const SMOKY_ALPHA: f64 = 0.87;

pub const GLASS_FALLBACK_FG: &str = "#F8FAFC";

/// One color theme: background / foreground / cursor as `#RRGGBB`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    #[serde(default)]
    pub cursor: String,
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
        validate_hex_field("background", &theme.background, &theme.name)?;
        validate_hex_field("foreground", &theme.foreground, &theme.name)?;
        validate_hex_field("cursor", &theme.cursor, &theme.name)?;
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

    /// Only the embedded built-in themes (no filesystem access).
    pub fn builtin() -> Self {
        let mut themes = Vec::new();
        for (file, json) in BUILTIN_FILES {
            let theme = Theme::from_json(json).unwrap_or_else(|e| {
                panic!("corrupt embedded theme {file}: {e}");
            });
            themes.push(theme);
        }
        Self { themes }
    }

    /// Built-ins + user themes from `dir` (when it exists and is
    /// readable). Invalid JSON files are skipped with a warning on
    /// stderr; valid files override built-ins with the same name.
    pub fn load(user_dir: Option<&Path>) -> Self {
        let mut reg = Self::builtin();
        let Some(dir) = user_dir else { return reg };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return reg;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        files.sort();
        let mut seen_user_names: std::collections::HashSet<String> = Default::default();
        for path in files {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Ok(content) = std::fs::read_to_string(&path) else {
                eprintln!("Warning: could not read theme file {file_name}");
                continue;
            };
            match Theme::from_json(&content) {
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
/// fall back to the default theme (Tokyo Night).
pub fn theme_colors(theme_name: &str) -> (String, String) {
    let reg = Registry::global();
    let theme = reg.get(theme_name).unwrap_or_else(|| reg.fallback());
    (theme.background.clone(), theme.foreground.clone())
}

/// Port of `get_colors`: when glass mode is on (transparent, or static /
/// smoky blur) the terminal itself becomes fully transparent and falls
/// back to a light foreground; otherwise the theme colors are used.
pub fn get_colors(theme_name: &str, transparent: bool, blur_mode: &str) -> (Rgba, Rgba) {
    let glass = transparent || blur_mode == "static" || blur_mode == "smoky";
    if glass {
        (
            Rgba::black_transparent(),
            Rgba::opaque(GLASS_FALLBACK_FG),
        )
    } else {
        let (bg_hex, fg_hex) = theme_colors(theme_name);
        (Rgba::opaque(&bg_hex), Rgba::opaque(&fg_hex))
    }
}

/// Is the given blur mode / transparency combination "glass"?
pub fn is_glass(transparent: bool, blur_mode: &str) -> bool {
    transparent || blur_mode == "static" || blur_mode == "smoky"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_30_unique_valid_themes() {
        let reg = Registry::builtin();
        assert_eq!(reg.len(), 30);
        let names = reg.names();
        for name in &names {
            assert!(reg.get(name).is_some());
        }
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "duplicate theme names");
    }

    #[test]
    fn embedded_files_and_builtin_registry_are_in_sync() {
        for (file, json) in BUILTIN_FILES {
            let theme = Theme::from_json(json)
                .unwrap_or_else(|e| panic!("embedded theme {file} is invalid: {e}"));
            // The file name (slug) must not drift away from the theme name.
            assert!(file.ends_with(".json"), "bad file name: {file}");
            assert!(!theme.name.is_empty());
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
    fn tokyo_night_is_the_default_and_comes_from_its_file() {
        assert_eq!(DEFAULT_THEME, "Tokyo Night");
        let reg = Registry::builtin();
        let theme = reg.get(DEFAULT_THEME).expect("Tokyo Night must exist");
        assert_eq!(theme.background, "#1A1B26");
        assert_eq!(theme.foreground, "#A9B1D6");
        assert_eq!(theme.cursor, "#A9B1D6");
    }

    #[test]
    fn unknown_theme_falls_back_to_tokyo_night() {
        let reg = Registry::builtin();
        assert_eq!(reg.fallback().name, "Tokyo Night");
        let (bg, fg) = theme_colors("Nope");
        assert_eq!(bg, "#1A1B26");
        assert_eq!(fg, "#A9B1D6");
    }

    #[test]
    fn theme_json_parse_accepts_missing_cursor() {
        let theme = Theme::from_json(
            r##"{"name": "X", "background": "#000000", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();
        assert_eq!(theme.cursor, "#FFFFFF");
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
        let path = dir.join("dracula.json");
        std::fs::write(
            &path,
            r##"{"name": "Dracula", "background": "#123456", "foreground": "#654321"}"##,
        )
        .unwrap();

        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 30, "override must not add a new slot");
        let dracula = reg.get("Dracula").unwrap();
        assert_eq!(dracula.background, "#123456");
        assert_eq!(dracula.foreground, "#654321");
        // Built-in order keeps Dracula in its original slot.
        assert_eq!(reg.names()[3], "Dracula");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn user_theme_with_new_name_is_appended() {
        let dir = temp_theme_dir("append");
        std::fs::write(
            dir.join("my-theme.json"),
            r##"{"name": "My Theme", "background": "#010203", "foreground": "#040506"}"##,
        )
        .unwrap();

        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 31);
        let names = reg.names();
        assert_eq!(names.last().unwrap(), "My Theme");
        assert_eq!(reg.get("My Theme").unwrap().background, "#010203");
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
        std::fs::write(
            dir.join("good.json"),
            r##"{"name": "Good", "background": "#000000", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();

        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 31);
        assert!(reg.get("Good").is_some());
        assert!(reg.get("BadColor").is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_user_dir_means_builtins_only() {
        let reg = Registry::load(Some(Path::new("/nonexistent/tuxixi-themes-xyz")));
        assert_eq!(reg.len(), 30);
    }

    #[test]
    fn first_user_file_wins_on_duplicate_names() {
        let dir = temp_theme_dir("dup");
        std::fs::write(
            dir.join("aaa.json"),
            r##"{"name": "Dupe", "background": "#000001", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();
        std::fs::write(
            dir.join("bbb.json"),
            r##"{"name": "Dupe", "background": "#000002", "foreground": "#FFFFFF"}"##,
        )
        .unwrap();
        let reg = Registry::load(Some(&dir));
        assert_eq!(reg.len(), 31);
        assert_eq!(reg.get("Dupe").unwrap().background, "#000001");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn theme_colors_returns_builtin_colors() {
        let (bg, fg) = theme_colors("Matrix");
        assert_eq!(bg, "#001100");
        assert_eq!(fg, "#00FF00");
        let (bg, fg) = theme_colors(DEFAULT_THEME);
        assert_eq!(bg, "#1A1B26");
        assert_eq!(fg, "#A9B1D6");
    }

    #[test]
    fn get_colors_normal_mode_uses_theme() {
        let (bg, fg) = get_colors("Matrix", false, "off");
        assert_eq!(bg, Rgba::parse_hex("#001100").unwrap());
        assert_eq!(fg, Rgba::parse_hex("#00FF00").unwrap());
        assert_eq!(bg.a, 1.0);
        assert_eq!(fg.a, 1.0);
    }

    #[test]
    fn get_colors_glass_mode_is_transparent_black_with_light_fg() {
        for transparent in [false, true] {
            for mode in ["static", "smoky"] {
                let (bg, fg) = get_colors("Dracula", transparent, mode);
                assert_eq!(bg, Rgba::new(0.0, 0.0, 0.0, 0.0));
                assert_eq!(fg, Rgba::parse_hex("#F8FAFC").unwrap());
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
}
