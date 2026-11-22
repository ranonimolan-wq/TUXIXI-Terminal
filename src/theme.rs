//! Port of `themes.py`: the 19 built-in themes, blur modes and color
//! resolution rules. No GTK dependency: colors are plain RGBA values.

use serde::{Deserialize, Serialize};

/// (background, foreground) hex pairs, in the exact order of `THEMES`
/// in themes.py (order matters: it defines the theme cycling sequence).
pub const THEME_LIST: &[(&str, (&str, &str))] = &[
    ("Matrix", ("#001100", "#00FF00")),
    ("Alacritty Dark", ("#1D1F21", "#C5C8C6")),
    ("Monokai", ("#272822", "#F8F8F2")),
    ("Dracula", ("#282A36", "#F8F8F2")),
    ("Nord", ("#2E3440", "#D8DEE9")),
    ("Gruvbox Dark", ("#282828", "#EBDBB2")),
    ("Solarized Dark", ("#002B36", "#839496")),
    ("One Dark", ("#282C34", "#ABB2BF")),
    ("Tokyo Night", ("#1A1B26", "#A9B1D6")),
    ("Catppuccin", ("#1E1E2E", "#CDD6F4")),
    ("Cyberpunk", ("#120B1E", "#00F0FF")),
    ("Classic Black/White", ("#000000", "#FFFFFF")),
    ("Classic White/Black", ("#FFFFFF", "#000000")),
    ("Solarized Light", ("#FDF6E3", "#657B83")),
    ("PaperColor Light", ("#EEEEEE", "#444444")),
    ("Gruvbox Light", ("#FBF1C7", "#3C3836")),
    ("Purple Haze", ("#1A0B2E", "#D8B4FE")),
    ("Ocean Breeze", ("#0F172A", "#38BDF8")),
    ("Retro Amber", ("#1C160C", "#FFB000")),
];

pub const THEME_NAMES: &[&str] = &[
    "Matrix",
    "Alacritty Dark",
    "Monokai",
    "Dracula",
    "Nord",
    "Gruvbox Dark",
    "Solarized Dark",
    "One Dark",
    "Tokyo Night",
    "Catppuccin",
    "Cyberpunk",
    "Classic Black/White",
    "Classic White/Black",
    "Solarized Light",
    "PaperColor Light",
    "Gruvbox Light",
    "Purple Haze",
    "Ocean Breeze",
    "Retro Amber",
];

pub const BLUR_MODES: &[&str] = &["off", "static", "smoky"];

pub const TRANSLUCENT_ALPHA: f64 = 0.52;
pub const SMOKY_ALPHA: f64 = 0.87;

pub const GLASS_FALLBACK_FG: &str = "#F8FAFC";

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

pub fn theme_colors(theme_name: &str) -> (&'static str, &'static str) {
    for (name, colors) in THEME_LIST {
        if *name == theme_name {
            return *colors;
        }
    }
    // Python: THEMES.get(theme_name, THEMES["Alacritty Dark"])
    ("#1D1F21", "#C5C8C6")
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
        (Rgba::opaque(bg_hex), Rgba::opaque(fg_hex))
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
    fn theme_count_is_19_and_names_match_list() {
        assert_eq!(THEME_LIST.len(), 19);
        assert_eq!(THEME_NAMES.len(), 19);
        for (i, (name, _)) in THEME_LIST.iter().enumerate() {
            assert_eq!(*name, THEME_NAMES[i]);
        }
    }

    #[test]
    fn theme_colors_are_valid_hex() {
        for (name, (bg, fg)) in THEME_LIST {
            assert!(Rgba::parse_hex(bg).is_some(), "bad bg for {name}");
            assert!(Rgba::parse_hex(fg).is_some(), "bad fg for {name}");
        }
    }

    #[test]
    fn unknown_theme_falls_back_to_alacritty_dark() {
        assert_eq!(theme_colors("Nope"), ("#1D1F21", "#C5C8C6"));
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
    fn constants_match_python() {
        assert_eq!(BLUR_MODES, &["off", "static", "smoky"]);
        assert!((TRANSLUCENT_ALPHA - 0.52).abs() < f64::EPSILON);
        assert!((SMOKY_ALPHA - 0.87).abs() < f64::EPSILON);
    }
}
