// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Settings persistence and change notification.
//!
//! [`SettingsManager`] owns the current [`Settings`] snapshot, loads
//! and atomically saves `config.json`, and broadcasts every actual
//! change to registered listeners.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::model::{
    Settings, LEGACY_DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MAX_GLASS_OPACITY, MIN_FONT_SIZE,
    MIN_GLASS_OPACITY,
};
use crate::theme::Registry;

pub type ChangeListener = std::rc::Rc<dyn Fn(&Settings)>;

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
                Ok(raw) => {
                    let data = Settings::validate_raw(&raw);
                    // Persist one-time migrations (e.g. the legacy 11px
                    // default font size) so the upgrade is stable and
                    // visible in config.json instead of being re-applied
                    // invisibly on every start.
                    if raw.get("font_size").and_then(|v| v.as_u64())
                        == Some(u64::from(LEGACY_DEFAULT_FONT_SIZE))
                    {
                        self.atomic_save(&data);
                    }
                    data
                }
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

    /// Register a listener invoked with the new [`Settings`] snapshot on
    /// every actual change (theme, glass mode, blur, font...). This is
    /// how the GUI re-applies its CSS and fonts live.
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
        let names = Registry::global().names();
        let current = &self.data.theme;
        let next = match names.iter().position(|t| t == current) {
            Some(i) => names[(i + 1) % names.len()].clone(),
            None => names[0].clone(),
        };
        let mut s = self.data.clone();
        s.theme = next;
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

    /// Flip the glass mode on/off (persisted). Returns whether changed.
    pub fn toggle_glass(&mut self) -> bool {
        let mut s = self.data.clone();
        s.glass = !s.glass;
        self.set(s)
    }

    /// Set the glass background opacity, clamped to the valid range.
    /// Returns whether anything changed (a clamped-but-equal value is
    /// also a no-op).
    pub fn set_glass_opacity(&mut self, opacity: f32) -> bool {
        if !opacity.is_finite() {
            return false;
        }
        let clamped = opacity.clamp(MIN_GLASS_OPACITY, MAX_GLASS_OPACITY);
        if (self.data.glass_opacity - clamped).abs() < f32::EPSILON {
            return false;
        }
        let mut s = self.data.clone();
        s.glass_opacity = clamped;
        self.set(s)
    }

    fn notify_all(&self) {
        for l in &self.listeners {
            l(&self.data);
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
    Err(std::io::Error::other("could not create temp file"))
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
