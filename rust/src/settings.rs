//! Port of `settings.py`: validated, atomically-saved settings stored in
//! `~/.config/tuxixi/config.json` (the path is injectable for tests).
//!
//! The GObject signal `settings-changed` is replaced by listener
//! callbacks registered with [`SettingsManager::on_change`].

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::theme::{theme_colors, BLUR_MODES, THEME_NAMES};

pub const DEFAULT_FONT: &str = "Monospace";
pub const DEFAULT_FONT_SIZE: u32 = 11;
pub const DEFAULT_THEME: &str = "Alacritty Dark";
pub const MIN_FONT_SIZE: u32 = 6;
pub const MAX_FONT_SIZE: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlurMode {
    Off,
    Static,
    Smoky,
}

impl BlurMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlurMode::Off => "off",
            BlurMode::Static => "static",
            BlurMode::Smoky => "smoky",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "off" => Some(BlurMode::Off),
            "static" => Some(BlurMode::Static),
            "smoky" => Some(BlurMode::Smoky),
            _ => None,
        }
    }

    /// `off -> static -> smoky -> off`
    pub fn next(&self) -> Self {
        match self {
            BlurMode::Off => BlurMode::Static,
            BlurMode::Static => BlurMode::Smoky,
            BlurMode::Smoky => BlurMode::Off,
        }
    }
}

impl fmt::Display for BlurMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub font: String,
    pub font_size: u32,
    pub theme: String,
    pub transparent: bool,
    pub blur_mode: BlurMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font: DEFAULT_FONT.to_string(),
            font_size: DEFAULT_FONT_SIZE,
            theme: DEFAULT_THEME.to_string(),
            transparent: false,
            blur_mode: BlurMode::Off,
        }
    }
}

impl Settings {
    /// Port of `SettingsManager._validate`: every invalid field silently
    /// falls back to its default, out-of-range font sizes are clamped.
    /// The legacy `"blur": true` key (pre 2.1 config format) maps to
    /// `static` blur when `blur_mode` itself is invalid.
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

        if let Some(v) = obj.get("theme") {
            if let Some(theme) = v.as_str() {
                if THEME_NAMES.contains(&theme) {
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
                if let Some(m) = BlurMode::from_str(mode) {
                    s.blur_mode = m;
                }
            }
        }

        // Legacy compatibility: {"blur": true} meant static blur.
        if !matches!(obj.get("blur_mode"), Some(serde_json::Value::String(_)))
            || s.blur_mode == BlurMode::Off
        {
            if obj.get("blur").and_then(|b| b.as_bool()) == Some(true) {
                // Python only applies the legacy key when blur_mode itself
                // was invalid; keep the same precedence.
                let mode_invalid = match obj.get("blur_mode") {
                    Some(serde_json::Value::String(m)) => BlurMode::from_str(m).is_none(),
                    _ => true,
                };
                if mode_invalid {
                    s.blur_mode = BlurMode::Static;
                }
            }
        }

        s
    }

    pub fn theme_colors(&self) -> (&'static str, &'static str) {
        theme_colors(&self.theme)
    }
}

pub type ChangeListener = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

pub struct SettingsManager {
    pub config_path: PathBuf,
    data: Settings,
    listeners: Vec<ChangeListener>,
}

impl SettingsManager {
    /// Load settings from `path`, creating the file with defaults when
    /// missing (exactly like the Python version).
    pub fn load(path: &Path) -> Self {
        let mut mgr = Self {
            config_path: path.to_path_buf(),
            data: Settings::default(),
            listeners: Vec::new(),
        };
        mgr.data = mgr.load_or_default();
        mgr
    }

    /// Load settings from `$XDG_DATA_HOME/../config/tuxixi/config.json`
    /// (i.e. `~/.config/tuxixi/config.json`).
    pub fn load_default_location() -> Self {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                Path::new(&home).join(".config")
            });
        Self::load(&config_dir.join("tuxixi").join("config.json"))
    }

    fn load_or_default(&self) -> Settings {
        match fs::read_to_string(&self.config_path) {
            Err(_) => {
                // Missing file: write defaults immediately (Python does
                // this on first load).
                let data = Settings::default();
                self.atomic_save(&data);
                data
            }
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(raw) => Settings::validate_raw(&raw),
                Err(e) => {
                    eprintln!("Warning: Failed to read config, using defaults: {e}");
                    Settings::default()
                }
            },
        }
    }

    /// Atomic save: write to a temp file in the same directory, fsync,
    /// then rename over the target. On any failure a warning is printed
    /// and the temp file is cleaned up (port of `_atomic_save`).
    fn atomic_save(&self, data: &Settings) {
        let dir = match self.config_path.parent() {
            Some(d) => d,
            None => return,
        };

        let result = (|| -> std::io::Result<()> {
            fs::create_dir_all(dir)?;
            let mut tmp = tempfile_in(dir)?;
            let json = serde_json::to_string_pretty(data)?;
            tmp.file.write_all(json.as_bytes())?;
            tmp.file.write_all(b"\n")?;
            tmp.file.sync_all()?;
            fs::rename(&tmp.path, &self.config_path)?;
            Ok(())
        })();

        if let Err(e) = result {
            eprintln!("Error: The configuration could not be saved: {e}");
            let _ = cleanup_temp_files(dir);
        }
    }

    pub fn get(&self) -> &Settings {
        &self.data
    }

    pub fn on_change(&mut self, listener: ChangeListener) {
        self.listeners.push(listener);
    }

    /// Set a value: no-op (no save, no notification) when unchanged,
    /// otherwise persist and notify listeners. Returns whether changed.
    pub fn set(&mut self, changed: Settings) -> bool {
        if self.data == changed {
            return false;
        }
        self.data = changed;
        self.atomic_save(&self.data);
        self.notify_all();
        true
    }

    pub fn change_font_size(&mut self, delta: i32) -> bool {
        let current = self.data.font_size as i32;
        let new_size = (current + delta).clamp(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32) as u32;
        let mut s = self.data.clone();
        s.font_size = new_size;
        self.set(s)
    }

    pub fn cycle_theme(&mut self) -> bool {
        let current = &self.data.theme;
        let next = match THEME_NAMES.iter().position(|t| t == current) {
            Some(i) => THEME_NAMES[(i + 1) % THEME_NAMES.len()],
            None => THEME_NAMES[0],
        };
        let mut s = self.data.clone();
        s.theme = next.to_string();
        self.set(s)
    }

    pub fn toggle_transparency(&mut self) -> bool {
        let mut s = self.data.clone();
        s.transparent = !s.transparent;
        self.set(s)
    }

    pub fn cycle_blur_mode(&mut self) -> bool {
        let mut s = self.data.clone();
        s.blur_mode = s.blur_mode.next();
        self.set(s)
    }

    fn notify_all(&self) {
        for l in &self.listeners {
            l("settings-changed");
        }
    }
}

struct TempGuard {
    path: PathBuf,
    file: fs::File,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        // If rename already happened this silently fails; fine.
        let _ = fs::remove_file(&self.path);
    }
}

fn tempfile_in(dir: &Path) -> std::io::Result<TempGuard> {
    use std::os::unix::fs::OpenOptionsExt;
    for _ in 0..5 {
        let name = format!(
            ".config.{}.tmp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
                + (std::process::id() as u128) % 1_000_000
        );
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok(TempGuard { path, file }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "could not create temp file",
    ))
}

fn cleanup_temp_files(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".config.") && name.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/// True when `theme` is a valid built-in theme name.
pub fn is_valid_theme(theme: &str) -> bool {
    THEME_NAMES.contains(&theme)
}

/// True when `mode` is a valid blur mode string.
pub fn is_valid_blur_mode(mode: &str) -> bool {
    BLUR_MODES.contains(&mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::THEME_NAMES;

    fn temp_config_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuxixi-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_match_python() {
        let s = Settings::default();
        assert_eq!(s.font, "Monospace");
        assert_eq!(s.font_size, 11);
        assert_eq!(s.theme, "Alacritty Dark");
        assert!(!s.transparent);
        assert_eq!(s.blur_mode, BlurMode::Off);
    }

    #[test]
    fn validate_rejects_garbage() {
        let raw = serde_json::json!("not an object");
        assert_eq!(Settings::validate_raw(&raw), Settings::default());
    }

    #[test]
    fn validate_clamps_font_size() {
        let raw = serde_json::json!({"font_size": 500});
        assert_eq!(Settings::validate_raw(&raw).font_size, MAX_FONT_SIZE);
        let raw = serde_json::json!({"font_size": 1});
        assert_eq!(Settings::validate_raw(&raw).font_size, MIN_FONT_SIZE);
        let raw = serde_json::json!({"font_size": 24});
        assert_eq!(Settings::validate_raw(&raw).font_size, 24);
        let raw = serde_json::json!({"font_size": true});
        assert_eq!(Settings::validate_raw(&raw).font_size, DEFAULT_FONT_SIZE);
        let raw = serde_json::json!({"font_size": "24"});
        assert_eq!(Settings::validate_raw(&raw).font_size, DEFAULT_FONT_SIZE);
    }

    #[test]
    fn validate_rejects_unknown_theme_and_trims_font() {
        let raw = serde_json::json!({"theme": "Not A Theme", "font": "  JetBrains Mono  "});
        let s = Settings::validate_raw(&raw);
        assert_eq!(s.theme, DEFAULT_THEME);
        assert_eq!(s.font, "JetBrains Mono");
    }

    #[test]
    fn validate_rejects_unknown_blur_mode_but_accepts_legacy_blur_true() {
        let raw = serde_json::json!({"blur_mode": "weird"});
        assert_eq!(Settings::validate_raw(&raw).blur_mode, BlurMode::Off);

        let raw = serde_json::json!({"blur": true});
        assert_eq!(Settings::validate_raw(&raw).blur_mode, BlurMode::Static);

        // Explicit valid blur_mode wins over the legacy key.
        let raw = serde_json::json!({"blur": true, "blur_mode": "smoky"});
        assert_eq!(Settings::validate_raw(&raw).blur_mode, BlurMode::Smoky);
    }

    #[test]
    fn load_missing_file_creates_defaults_file() {
        let dir = temp_config_dir("missing");
        let path = dir.join("config.json");
        assert!(!path.exists());
        let mgr = SettingsManager::load(&path);
        assert_eq!(mgr.get(), &Settings::default());
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["theme"], "Alacritty Dark");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_invalid_json_falls_back_to_defaults() {
        let dir = temp_config_dir("badjson");
        let path = dir.join("config.json");
        fs::write(&path, "{{{ not json").unwrap();
        let mgr = SettingsManager::load(&path);
        assert_eq!(mgr.get(), &Settings::default());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_roundtrip_preserves_values() {
        let dir = temp_config_dir("roundtrip");
        let path = dir.join("config.json");
        let mut mgr = SettingsManager::load(&path);
        let mut s = mgr.get().clone();
        s.font = "Fira Code".into();
        s.font_size = 17;
        s.theme = "Dracula".into();
        s.transparent = true;
        s.blur_mode = BlurMode::Smoky;
        assert!(mgr.set(s.clone()));
        assert_eq!(mgr.get(), &s);

        let mgr2 = SettingsManager::load(&path);
        assert_eq!(mgr2.get(), &s);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn set_same_value_is_noop_and_no_notification() {
        let dir = temp_config_dir("noop");
        let path = dir.join("config.json");
        let mut mgr = SettingsManager::load(&path);
        let notified = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n2 = notified.clone();
        mgr.on_change(std::sync::Arc::new(move |_| {
            n2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        assert!(!mgr.set(mgr.get().clone()));
        assert_eq!(notified.load(std::sync::atomic::Ordering::SeqCst), 0);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn font_size_cycles_are_clamped() {
        let dir = temp_config_dir("fontsize");
        let path = dir.join("config.json");
        let mut mgr = SettingsManager::load(&path);
        // 11 - 100 clamps to 6: a change, so it returns true.
        assert!(mgr.change_font_size(-100));
        assert_eq!(mgr.get().font_size, MIN_FONT_SIZE);
        // Already at the minimum: clamped to the same value -> no change.
        assert!(!mgr.change_font_size(-100));
        assert!(mgr.change_font_size(100));
        assert_eq!(mgr.get().font_size, MAX_FONT_SIZE);
        // Already at the maximum: +1 is a no-op.
        assert!(!mgr.change_font_size(1));
        assert!(mgr.change_font_size(-1));
        assert_eq!(mgr.get().font_size, MAX_FONT_SIZE - 1);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn theme_cycle_wraps_around() {
        let dir = temp_config_dir("themecycle");
        let path = dir.join("config.json");
        let mut mgr = SettingsManager::load(&path);

        // Default is "Alacritty Dark" (index 1) -> cycles to index 2.
        assert!(mgr.cycle_theme());
        assert_eq!(mgr.get().theme, THEME_NAMES[2]);

        // Jump to the last theme and cycle: must wrap to the first.
        let mut s = mgr.get().clone();
        s.theme = THEME_NAMES[THEME_NAMES.len() - 1].to_string();
        mgr.set(s);
        assert!(mgr.cycle_theme());
        assert_eq!(mgr.get().theme, THEME_NAMES[0]);

        // Invalid theme resets to the first (Python: next_index = 0).
        let mut s = mgr.get().clone();
        s.theme = "Bogus".into();
        mgr.set(s);
        assert!(mgr.cycle_theme());
        assert_eq!(mgr.get().theme, THEME_NAMES[0]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn blur_cycle_is_off_static_smoky_off() {
        let dir = temp_config_dir("blurcycle");
        let path = dir.join("config.json");
        let mut mgr = SettingsManager::load(&path);
        assert_eq!(mgr.get().blur_mode, BlurMode::Off);
        mgr.cycle_blur_mode();
        assert_eq!(mgr.get().blur_mode, BlurMode::Static);
        mgr.cycle_blur_mode();
        assert_eq!(mgr.get().blur_mode, BlurMode::Smoky);
        mgr.cycle_blur_mode();
        assert_eq!(mgr.get().blur_mode, BlurMode::Off);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transparency_toggles() {
        let dir = temp_config_dir("toggle");
        let path = dir.join("config.json");
        let mut mgr = SettingsManager::load(&path);
        assert!(!mgr.get().transparent);
        mgr.toggle_transparency();
        assert!(mgr.get().transparent);
        mgr.toggle_transparency();
        assert!(!mgr.get().transparent);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_save_leaves_no_temp_files_behind() {
        let dir = temp_config_dir("atomic");
        let path = dir.join("tuxixi").join("config.json");
        let mut mgr = SettingsManager::load(&path);
        mgr.toggle_transparency();
        let files: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "config.json");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn blur_mode_next_wraps() {
        assert_eq!(BlurMode::Off.next(), BlurMode::Static);
        assert_eq!(BlurMode::Static.next(), BlurMode::Smoky);
        assert_eq!(BlurMode::Smoky.next(), BlurMode::Off);
    }

    #[test]
    fn validity_helpers() {
        assert!(is_valid_theme("Dracula"));
        assert!(!is_valid_theme("dracula"));
        assert!(is_valid_blur_mode("smoky"));
        assert!(!is_valid_blur_mode("SMOKY"));
    }
}
