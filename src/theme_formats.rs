//! Readers for the REAL upstream theme formats.
//!
//! TUXIXI does not require themes to be converted to its own JSON
//! format first. It reads the files their original creators publish:
//!
//! * iTerm2 / iTerm2-Color-Schemes  `.itermcolors` (XML property list)
//! * Alacritty                      `.toml` and legacy `.yml`/`.yaml`
//! * kitty                          `.conf`
//! * TUXIXI native                  `.json`
//!
//! Any of these dropped into `~/.config/tuxixi/themes/` (or a whole
//! cloned theme repository, scanned recursively) becomes a usable
//! theme. Only the subset of each format that describes colors is
//! parsed; everything else is ignored. A theme is identified by its
//! file stem: dashes become spaces (`TokyoNight-Storm` ->
//! `TokyoNight Storm`, `tokyo-night` -> `Tokyo Night`).

use std::path::Path;

use crate::theme::Theme;

/// File extensions recognized by [`theme_from_file`].
pub const SUPPORTED_EXTENSIONS: &[&str] = &["itermcolors", "json", "toml", "yml", "yaml", "conf"];

/// Turn a file stem into a display name: dashes become spaces. Word
/// capitalization only applies to lowercase words, so `tokyo-night` ->
/// `Tokyo Night` while `TokyoNight-Storm` -> `TokyoNight Storm` and
/// `iTerm2-Solarized-Dark` -> `iTerm2 Solarized Dark` keep their
/// original casing.
pub fn name_from_stem(stem: &str) -> String {
    stem.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) if first.is_lowercase() => {
                    // Capitalize, unless the word is CamelCase (iTerm2).
                    let rest: String = chars.collect();
                    let second_upper = rest.chars().next().map(char::is_uppercase).unwrap_or(false);
                    if second_upper {
                        w.to_string()
                    } else {
                        first.to_uppercase().collect::<String>() + rest.as_str()
                    }
                }
                _ => w.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a theme file. `path` provides the display name (file stem).
/// Returns `Err` with a reason when the file cannot be understood.
pub fn theme_from_file(path: &Path, content: &str) -> Result<Theme, String> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "theme".to_string());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name = name_from_stem(&stem);

    let mut theme = match ext.as_str() {
        "json" => Theme::from_json(content).map_err(|e| format!("invalid theme JSON: {e}"))?,
        "itermcolors" => parse_itermcolors(content)?,
        "toml" => parse_alacritty_toml(content)?,
        "yml" | "yaml" => parse_alacritty_yaml(content)?,
        "conf" => parse_kitty_conf(content)?,
        other => return Err(format!("unsupported theme extension: {other}")),
    };

    // The file stem wins over whatever the content may claim (most
    // upstream formats have no name field at all; a JSON file whose
    // "name" disagrees with the file name would be confusing).
    theme.name = name;
    if theme.cursor.trim().is_empty() {
        theme.cursor = theme.foreground.clone();
    }
    Theme::from_json(
        &serde_json::to_string(&theme).map_err(|e| format!("cannot serialize theme: {e}"))?,
    )
    .map_err(|e| format!("theme validation failed: {e}"))
}

// ----------------------------------------------------------------------
// iTerm2 .itermcolors (XML plist, float components)
// ----------------------------------------------------------------------

fn comp_to_hex(components: [Option<f64>; 3]) -> Option<String> {
    let [r, g, b] = components;
    match (r, g, b) {
        (Some(r), Some(g), Some(b)) => {
            // Plists store 0.0-1.0 floats; be tolerant of 0-255 scales.
            let scale = if r > 1.0 || g > 1.0 || b > 1.0 { 255.0 } else { 1.0 };
            let to_byte = |v: f64| ((v / scale) * 255.0).round().clamp(0.0, 255.0) as u8;
            Some(format!("#{:02X}{:02X}{:02X}", to_byte(r), to_byte(g), to_byte(b)))
        }
        _ => None,
    }
}

/// Known iTerm2 color slots, in palette order.
const ITERM_KEYS: [(&str, Slot); 19] = [
    ("Ansi 0 Color", Slot::Palette(0)),
    ("Ansi 1 Color", Slot::Palette(1)),
    ("Ansi 2 Color", Slot::Palette(2)),
    ("Ansi 3 Color", Slot::Palette(3)),
    ("Ansi 4 Color", Slot::Palette(4)),
    ("Ansi 5 Color", Slot::Palette(5)),
    ("Ansi 6 Color", Slot::Palette(6)),
    ("Ansi 7 Color", Slot::Palette(7)),
    ("Ansi 8 Color", Slot::Palette(8)),
    ("Ansi 9 Color", Slot::Palette(9)),
    ("Ansi 10 Color", Slot::Palette(10)),
    ("Ansi 11 Color", Slot::Palette(11)),
    ("Ansi 12 Color", Slot::Palette(12)),
    ("Ansi 13 Color", Slot::Palette(13)),
    ("Ansi 14 Color", Slot::Palette(14)),
    ("Ansi 15 Color", Slot::Palette(15)),
    ("Background Color", Slot::Background),
    ("Foreground Color", Slot::Foreground),
    ("Cursor Color", Slot::Cursor),
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Slot {
    Palette(usize),
    Background,
    Foreground,
    Cursor,
}

/// Line-based parser for the fixed iTerm2 plist structure:
/// `<key>NAME</key>` + `<dict>` with `<key>Red Component</key><real>v</real>`
/// pairs. No XML dependency needed for this well-known shape.
pub fn parse_itermcolors(content: &str) -> Result<Theme, String> {
    let mut colors: Vec<(Slot, String)> = Vec::new();
    let mut current: Option<Slot> = None;
    let mut comps: [Option<f64>; 3] = [None, None, None];
    let mut comp_idx: Option<usize> = None;
    let mut saw_dict = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if let Some(name) = line.strip_prefix("<key>").and_then(|l| l.strip_suffix("</key>")) {
            if let Some((_, slot)) = ITERM_KEYS.iter().find(|(k, _)| *k == name) {
                flush_slot(&mut colors, &mut comps, current);
                current = Some(*slot);
                comps = [None, None, None];
                comp_idx = None;
                saw_dict = false;
            } else if let Some(slot) = current {
                comp_idx = match name {
                    "Red Component" => Some(0),
                    "Green Component" => Some(1),
                    "Blue Component" => Some(2),
                    _ => None,
                };
                let _ = slot;
            }
            continue;
        }
        if line.starts_with("<dict>") {
            saw_dict = true;
            continue;
        }
        if line.starts_with("</dict>") {
            flush_slot(&mut colors, &mut comps, current);
            current = None;
            comp_idx = None;
            saw_dict = false;
            continue;
        }
        if let (Some(idx), Some(_)) = (comp_idx, current) {
            // Value lines: <real>0.15</real>, <integer>15</integer> etc.
            if let Some(value) = extract_numeric(line) {
                comps[idx] = Some(value);
                comp_idx = None;
            }
        }
        let _ = saw_dict;
    }
    flush_slot(&mut colors, &mut comps, current);

    let take = |slot: Slot| colors.iter().find(|(s, _)| *s == slot).map(|(_, c)| c.clone());
    let background = take(Slot::Background)
        .ok_or_else(|| "itermcolors file has no Background Color".to_string())?;
    let foreground = take(Slot::Foreground)
        .ok_or_else(|| "itermcolors file has no Foreground Color".to_string())?;
    let cursor = take(Slot::Cursor).unwrap_or_default();
    let mut palette = Vec::new();
    for i in 0..16 {
        palette.push(
            take(Slot::Palette(i))
                .ok_or_else(|| format!("itermcolors file is missing Ansi {i} Color"))?,
        );
    }

    Ok(Theme {
        name: String::new(),
        background,
        foreground,
        cursor,
        palette,
    })
}

fn flush_slot(colors: &mut Vec<(Slot, String)>, comps: &mut [Option<f64>; 3], current: Option<Slot>) {
    if let Some(slot) = current {
        if let Some(hex) = comp_to_hex(*comps) {
            colors.push((slot, hex));
        }
    }
    *comps = [None, None, None];
}

fn extract_numeric(line: &str) -> Option<f64> {
    let inner = line
        .strip_prefix("<real>")
        .and_then(|l| l.strip_suffix("</real>"))
        .or_else(|| {
            line.strip_prefix("<integer>")
                .and_then(|l| l.strip_suffix("</integer>"))
        })?;
    inner.trim().parse::<f64>().ok()
}

// ----------------------------------------------------------------------
// Alacritty .toml
// ----------------------------------------------------------------------

/// Minimal parser for the Alacritty `[colors]` schema. Only the color
/// tables are read; unknown keys and tables are ignored.
pub fn parse_alacritty_toml(content: &str) -> Result<Theme, String> {
    let mut background = String::new();
    let mut foreground = String::new();
    let mut cursor = String::new();
    let mut palette: Vec<String> = Vec::new();
    let mut normal: Vec<String> = vec![String::new(); 8];
    let mut bright: Vec<String> = vec![String::new(); 8];
    let mut section = String::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        let Some((key, value_raw)) = split_kv(line, '=') else {
            continue;
        };
        let Some(value) = strip_value(&value_raw) else {
            continue;
        };
        match (section.as_str(), key.as_str()) {
            ("colors.primary", "background") => background = value,
            ("colors.primary", "foreground") => foreground = value,
            ("colors.cursor", "cursor") => cursor = value,
            ("colors.normal", k) => {
                if let Some(i) = ansi_index(&k) {
                    normal[i] = value;
                }
            }
            ("colors.bright", k) => {
                if let Some(i) = ansi_index(&k) {
                    bright[i] = value;
                }
            }
            _ => {}
        }
    }

    if background.is_empty() || foreground.is_empty() {
        return Err("toml file has no colors.primary background/foreground".to_string());
    }
    palette.extend(normal.iter().cloned());
    palette.extend(bright.iter().cloned());
    palette.retain(|c| !c.is_empty());

    Ok(Theme {
        name: String::new(),
        background,
        foreground,
        cursor,
        palette,
    })
}

fn ansi_index(key: &str) -> Option<usize> {
    const NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    NAMES.iter().position(|n| *n == key)
}

fn split_kv(line: &str, sep: char) -> Option<(String, String)> {
    let idx = line.find(sep)?;
    let key = line[..idx].trim().trim_matches('"').to_string();
    let value = line[idx + sep.len_utf8()..].trim().to_string();
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key, value))
}

/// Extract a quoted value (TOML/YAML colors are always quoted), or, for
/// unquoted values, strip the trailing `# comment`. Quoted values keep
/// their `#` intact - `background = "#1d2021"` is a color, not a comment.
fn strip_value(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(inside) = raw.strip_prefix('"') {
        let end = inside.find('"')?;
        let value = inside[..end].trim();
        return if value.is_empty() { None } else { Some(value.to_string()) };
    }
    if let Some(inside) = raw.strip_prefix('\'') {
        let end = inside.find('\'')?;
        let value = inside[..end].trim();
        return if value.is_empty() { None } else { Some(value.to_string()) };
    }
    let value = raw.split('#').next().unwrap_or("").trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

// ----------------------------------------------------------------------
// Alacritty legacy .yml / .yaml
// ----------------------------------------------------------------------

/// Minimal parser for the legacy Alacritty YAML schema (fixed shape:
/// `colors:` at indent 0, `primary/normal/bright/cursor` at indent 2,
/// color keys at indent 4).
pub fn parse_alacritty_yaml(content: &str) -> Result<Theme, String> {
    let mut background = String::new();
    let mut foreground = String::new();
    let mut cursor = String::new();
    let mut palette: Vec<String> = Vec::new();
    let mut normal: Vec<String> = vec![String::new(); 8];
    let mut bright: Vec<String> = vec![String::new(); 8];
    let mut inside_colors = false;
    let mut section = String::new();

    for raw_line in content.lines() {
        let trimmed = raw_line.trim_end();
        let line = trimmed.trim();
        if line.is_empty() || line == "---" || line.starts_with('#') {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        if indent == 0 {
            inside_colors = line.trim_end_matches(':').trim() == "colors";
            section.clear();
            continue;
        }
        if !inside_colors {
            continue;
        }
        if indent <= 2 {
            section = line.trim_end_matches(':').to_string();
            continue;
        }
        let Some((key, value_raw)) = split_kv(line, ':') else {
            continue;
        };
        let Some(value) = strip_value(&value_raw) else {
            continue;
        };
        match (section.as_str(), key.as_str()) {
            ("primary", "background") => background = value,
            ("primary", "foreground") => foreground = value,
            ("cursor", "cursor") => cursor = value,
            ("normal", k) => {
                if let Some(i) = ansi_index(&k) {
                    normal[i] = value;
                }
            }
            ("bright", k) => {
                if let Some(i) = ansi_index(&k) {
                    bright[i] = value;
                }
            }
            _ => {}
        }
    }

    if background.is_empty() || foreground.is_empty() {
        return Err("yaml file has no colors.primary background/foreground".to_string());
    }
    for c in normal.iter().chain(bright.iter()) {
        if !c.is_empty() {
            palette.push(c.clone());
        }
    }

    Ok(Theme {
        name: String::new(),
        background,
        foreground,
        cursor,
        palette,
    })
}

// ----------------------------------------------------------------------
// kitty .conf
// ----------------------------------------------------------------------

/// Parser for kitty theme conf files (`background  #1d2021`,
/// `color1  #cc241d`, `# comments`).
pub fn parse_kitty_conf(content: &str) -> Result<Theme, String> {
    let mut background = String::new();
    let mut foreground = String::new();
    let mut cursor = String::new();
    let mut palette = vec![String::new(); 16];

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("map ") {
            continue;
        }
        let Some((key, value)) = split_kv(line, ' ') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.as_str() {
            "background" => background = value,
            "foreground" => foreground = value,
            "cursor" => cursor = value,
            k => {
                if let Some(num) = k.strip_prefix("color") {
                    if let Ok(i) = num.parse::<usize>() {
                        if i < 16 {
                            palette[i] = value;
                        }
                    }
                }
            }
        }
    }

    if background.is_empty() || foreground.is_empty() {
        return Err("conf file has no background/foreground".to_string());
    }
    palette.retain(|c| !c.is_empty());

    Ok(Theme {
        name: String::new(),
        background,
        foreground,
        cursor,
        palette,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn upstream(file: &str) -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        fs::read_to_string(Path::new(manifest_dir).join("themes/upstream").join(file))
            .unwrap_or_else(|e| panic!("cannot read upstream theme {file}: {e}"))
    }

    #[test]
    fn stem_to_name_variants() {
        assert_eq!(name_from_stem("TokyoNight-Storm"), "TokyoNight Storm");
        assert_eq!(name_from_stem("tokyo-night"), "Tokyo Night");
        assert_eq!(name_from_stem("nord"), "Nord");
        assert_eq!(name_from_stem("dracula"), "Dracula");
        assert_eq!(name_from_stem("iTerm2-Solarized-Dark"), "iTerm2 Solarized Dark");
        assert_eq!(name_from_stem("GitHub-Dark"), "GitHub Dark");
    }

    #[test]
    fn parses_the_real_tokyonight_upstream_file() {
        let content = upstream("TokyoNight.itermcolors");
        let theme = theme_from_file(Path::new("TokyoNight.itermcolors"), &content).unwrap();
        assert_eq!(theme.name, "TokyoNight");
        assert_eq!(theme.background, "#1A1B26");
        assert_eq!(theme.foreground, "#C0CAF5");
        assert_eq!(theme.cursor, "#C0CAF5");
        assert_eq!(theme.palette.len(), 16);
        // Ansi 1 (red) of TokyoNight is #F7768E.
        assert_eq!(theme.palette[1], "#F7768E");
    }

    #[test]
    fn parses_every_vendored_upstream_theme() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes/upstream");
        let mut count = 0;
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().map(|e| e != "itermcolors").unwrap_or(true) {
                continue;
            }
            let content = fs::read_to_string(&path).unwrap();
            let theme = theme_from_file(&path, &content)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(!theme.name.is_empty());
            assert_eq!(theme.palette.len(), 16, "{}", theme.name);
            count += 1;
        }
        assert!(count >= 30, "expected >=30 vendored themes, found {count}");
    }

    #[test]
    fn parses_alacritty_toml() {
        let toml = r##"
# Alacritty theme
[colors.primary]
background = "#1d2021"
foreground = "#fbf1c7"

[colors.cursor]
cursor = "#fbf1c7"

[colors.normal]
black   = "#282828"
red     = "#cc241d"
green   = "#98971a"
yellow  = "#d79921"
blue    = "#458588"
magenta = "#b16286"
cyan    = "#689d6a"
white   = "#a89984"

[colors.bright]
black   = "#928374"
red     = "#fb4934"
green   = "#b8bb26"
yellow  = "#fabd2f"
blue    = "#83a598"
magenta = "#d3869b"
cyan    = "#8ec07c"
white   = "#ebdbb2"
"##;
        let theme = theme_from_file(Path::new("gruvbox-dark.toml"), toml).unwrap();
        assert_eq!(theme.name, "Gruvbox Dark");
        assert_eq!(theme.background, "#1d2021");
        assert_eq!(theme.foreground, "#fbf1c7");
        assert_eq!(theme.cursor, "#fbf1c7");
        assert_eq!(theme.palette.len(), 16);
        assert_eq!(theme.palette[0], "#282828");
        assert_eq!(theme.palette[9], "#fb4934");
    }

    #[test]
    fn parses_alacritty_yaml() {
        let yaml = r##"
colors:
  primary:
    background: '#1d2021'
    foreground: '#fbf1c7'
  cursor:
    cursor: '#fabd2f'
  normal:
    black: '#282828'
    red: '#cc241d'
    green: '#98971a'
    yellow: '#d79921'
    blue: '#458588'
    magenta: '#b16286'
    cyan: '#689d6a'
    white: '#a89984'
  bright:
    black: '#928374'
    red: '#fb4934'
    green: '#b8bb26'
    yellow: '#fabd2f'
    blue: '#83a598'
    magenta: '#d3869b'
    cyan: '#8ec07c'
    white: '#ebdbb2'
"##;
        let theme = theme_from_file(Path::new("gruvbox-dark.yml"), yaml).unwrap();
        assert_eq!(theme.name, "Gruvbox Dark");
        assert_eq!(theme.background, "#1d2021");
        assert_eq!(theme.palette.len(), 16);
        assert_eq!(theme.palette[3], "#d79921");
    }

    #[test]
    fn parses_kitty_conf() {
        let conf = "
# Gruvbox dark for kitty
background            #1d2021
foreground            #fbf1c7
cursor                #fabd2f
selection_background  #fbf1c7

color0  #282828
color1  #cc241d
color2  #98971a
color3  #d79921
color4  #458588
color5  #b16286
color6  #689d6a
color7  #a89984
color8  #928374
color9  #fb4934
color10 #b8bb26
color11 #fabd2f
color12 #83a598
color13 #d3869b
color14 #8ec07c
color15 #ebdbb2
";
        let theme = theme_from_file(Path::new("gruvbox-dark.conf"), conf).unwrap();
        assert_eq!(theme.name, "Gruvbox Dark");
        assert_eq!(theme.background, "#1d2021");
        assert_eq!(theme.cursor, "#fabd2f");
        assert_eq!(theme.palette.len(), 16);
        assert_eq!(theme.palette[11], "#fabd2f");
    }

    #[test]
    fn json_themes_still_work_with_the_file_pipeline() {
        let json = r##"{"name": "Custom", "background": "#000000", "foreground": "#FFFFFF"}"##;
        let theme = theme_from_file(Path::new("custom.json"), json).unwrap();
        assert_eq!(theme.name, "Custom"); // stem, not the JSON name
        assert_eq!(theme.background, "#000000");
    }

    #[test]
    fn garbage_returns_helpful_errors() {
        assert!(theme_from_file(Path::new("x.itermcolors"), "not xml").is_err());
        assert!(theme_from_file(Path::new("x.toml"), "no colors here").is_err());
        assert!(theme_from_file(Path::new("x.conf"), "background #000").is_err());
        assert!(theme_from_file(Path::new("x.docx"), "???").is_err());
    }
}
