// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Unit tests for the settings model, manager and persistence.
//!
//! The whole file is compiled only under `cfg(test)`: the parent module
//! declares it via `#[cfg(test)] mod tests;`.

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::theme::{Registry, DEFAULT_THEME};

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
fn defaults_are_sized_like_alacritty() {
    let s = Settings::default();
    assert_eq!(s.font, "Monospace");
    assert_eq!(s.font_size, 15);
    assert_eq!(DEFAULT_FONT_SIZE, 15);
    assert_eq!(LEGACY_DEFAULT_FONT_SIZE, 11);
    // The migration must never fire on the new default itself.
    assert_ne!(DEFAULT_FONT_SIZE, LEGACY_DEFAULT_FONT_SIZE);
    assert_eq!(s.theme, "TokyoNight");
    assert!(!s.transparent);
    assert_eq!(s.blur_mode, BlurMode::Off);
    assert!(s.glass, "glass blur is on by default");
    assert_eq!(s.glass_opacity, DEFAULT_GLASS_OPACITY);
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
    // The old default (11) is exactly the migration trigger.
    let raw = serde_json::json!({"font_size": LEGACY_DEFAULT_FONT_SIZE});
    assert_eq!(Settings::validate_raw(&raw).font_size, DEFAULT_FONT_SIZE);
    // Custom (non-legacy) sizes pass through untouched.
    let raw = serde_json::json!({"font_size": 12});
    assert_eq!(Settings::validate_raw(&raw).font_size, 12);
    let raw = serde_json::json!({"font_size": 19});
    assert_eq!(Settings::validate_raw(&raw).font_size, 19);
    let raw = serde_json::json!({"font_size": true});
    assert_eq!(Settings::validate_raw(&raw).font_size, DEFAULT_FONT_SIZE);
    let raw = serde_json::json!({"font_size": "24"});
    assert_eq!(Settings::validate_raw(&raw).font_size, DEFAULT_FONT_SIZE);
}

#[test]
fn legacy_font_size_config_migrates_once_and_custom_sizes_survive_reloads() {
    let dir = temp_config_dir("fontmigration");
    let path = dir.join("config.json");

    // 1) A config written by an older rolling build (old default 11).
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        &path,
        r#"{"font": "Monospace", "font_size": 11, "theme": "TokyoNight"}"#,
    )
    .unwrap();
    let mgr = SettingsManager::load(&path);
    assert_eq!(
        mgr.get().font_size,
        DEFAULT_FONT_SIZE,
        "saved legacy default 11 must load as the new default"
    );
    // The migration is persisted so it is stable across restarts.
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        saved.get("font_size").and_then(|v| v.as_u64()),
        Some(u64::from(DEFAULT_FONT_SIZE)),
        "migrated size is written back to config.json"
    );

    // 2) A deliberately custom size must NOT be touched by the
    //    migration on any number of reloads.
    let mut mgr = mgr;
    mgr.change_font_size(-3); // 15 - 3 = 12 (custom, non-legacy)
    assert_eq!(mgr.get().font_size, 12);
    let reloaded = SettingsManager::load(&path);
    assert_eq!(reloaded.get().font_size, 12, "custom size survives reload");

    fs::remove_dir_all(dir).ok();
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
    assert_eq!(parsed["theme"], "TokyoNight");
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
    mgr.on_change(std::rc::Rc::new(move |_s| {
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
    // 15 - 100 clamps to 6: a change, so it returns true.
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
fn listener_receives_the_new_settings_snapshot() {
    let dir = temp_config_dir("listener");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    mgr.on_change(std::rc::Rc::new(move |s| {
        seen2
            .borrow_mut()
            .push(format!("{}@{}", s.theme, s.font_size));
    }));

    mgr.cycle_theme();
    mgr.change_font_size(2);
    let log = seen.borrow();
    assert_eq!(log.len(), 2, "one notification per actual change");
    assert!(
        log[0].ends_with("@15"),
        "font unchanged by theme cycle: {}",
        log[0]
    );
    assert!(
        log[1].ends_with("@17"),
        "new font size delivered: {}",
        log[1]
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn theme_cycle_wraps_around() {
    let dir = temp_config_dir("themecycle");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);

    // cycle_theme walks the GLOBAL registry (built-ins first, then
    // any user themes found on this machine). Compute the expected
    // sequence from the same source so the test is deterministic
    // regardless of the local user theme directory.
    let names = Registry::global().names();
    let default_pos = names
        .iter()
        .position(|n| n == crate::theme::DEFAULT_THEME)
        .expect("default theme must be in the registry");

    // Default theme cycles to whatever follows it in the list.
    assert!(mgr.cycle_theme());
    assert_eq!(mgr.get().theme, names[(default_pos + 1) % names.len()]);

    // Jump to the last theme and cycle: must wrap to the first.
    let mut s = mgr.get().clone();
    s.theme = names[names.len() - 1].clone();
    mgr.set(s);
    assert!(mgr.cycle_theme());
    assert_eq!(mgr.get().theme, names[0]);

    // Invalid theme resets to the first registry entry.
    let mut s = mgr.get().clone();
    s.theme = "Bogus".into();
    mgr.set(s);
    assert!(mgr.cycle_theme());
    assert_eq!(mgr.get().theme, names[0]);
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
fn glass_fields_validate_and_clamp() {
    let raw = serde_json::json!({"glass": false, "glass_opacity": 0.5});
    let s = Settings::validate_raw(&raw);
    assert!(!s.glass);
    assert_eq!(s.glass_opacity, 0.5);

    // Out of range values clamp into [MIN, MAX].
    let raw = serde_json::json!({"glass_opacity": 5.0});
    assert_eq!(
        Settings::validate_raw(&raw).glass_opacity,
        MAX_GLASS_OPACITY
    );
    let raw = serde_json::json!({"glass_opacity": 0.0});
    assert_eq!(
        Settings::validate_raw(&raw).glass_opacity,
        MIN_GLASS_OPACITY
    );

    // Non-numbers, booleans, NaN and inf all fall back to default.
    for junk in ["0.9", "true", "NaN"] {
        let raw = serde_json::json!({"glass_opacity": junk});
        assert_eq!(
            Settings::validate_raw(&raw).glass_opacity,
            DEFAULT_GLASS_OPACITY,
            "junk value {junk} must be rejected"
        );
    }
}

#[test]
fn glass_opacity_rejects_nan_and_inf() {
    let raw = serde_json::json!({"glass_opacity": 1.0f64 / 0.0f64});
    assert_eq!(
        Settings::validate_raw(&raw).glass_opacity,
        DEFAULT_GLASS_OPACITY
    );
    let raw = serde_json::json!({"glass_opacity": -(1.0f64 / 0.0f64)});
    assert_eq!(
        Settings::validate_raw(&raw).glass_opacity,
        DEFAULT_GLASS_OPACITY
    );
    let raw = serde_json::json!({"glass_opacity": serde_json::Value::Null});
    assert_eq!(
        Settings::validate_raw(&raw).glass_opacity,
        DEFAULT_GLASS_OPACITY
    );
    let raw = serde_json::json!({"glass_opacity": true});
    assert_eq!(
        Settings::validate_raw(&raw).glass_opacity,
        DEFAULT_GLASS_OPACITY
    );
}

#[test]
fn glass_toggles_and_persists() {
    let dir = temp_config_dir("glass");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);
    assert!(mgr.get().glass);
    // A toggle always flips the value, so it is always a change.
    assert!(mgr.toggle_glass());
    assert!(!mgr.get().glass);
    assert!(mgr.toggle_glass());
    assert!(mgr.get().glass);

    assert!(mgr.set_glass_opacity(0.55));
    assert_eq!(mgr.get().glass_opacity, 0.55);
    // Same value is a no-op.
    assert!(!mgr.set_glass_opacity(0.55));
    // Out-of-range values clamp instead of storing garbage.
    assert!(mgr.set_glass_opacity(42.0));
    assert_eq!(mgr.get().glass_opacity, MAX_GLASS_OPACITY);
    // NaN / inf are ignored entirely.
    assert!(!mgr.set_glass_opacity(f32::NAN));
    assert!(!mgr.set_glass_opacity(f32::INFINITY));
    assert_eq!(mgr.get().glass_opacity, MAX_GLASS_OPACITY);

    // Reload: everything persisted.
    let mgr2 = SettingsManager::load(&path);
    assert!(mgr2.get().glass);
    assert_eq!(mgr2.get().glass_opacity, MAX_GLASS_OPACITY);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn glass_opacity_survives_roundtrip() {
    let dir = temp_config_dir("glassrt");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);
    let mut s = mgr.get().clone();
    s.glass = false;
    s.glass_opacity = 0.33;
    assert!(mgr.set(s.clone()));
    let mgr2 = SettingsManager::load(&path);
    assert_eq!(mgr2.get(), &s);
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
    assert!(is_valid_theme("TokyoNight"));
    assert!(!is_valid_theme("dracula"));
    assert!(!is_valid_theme("Nope"));
    assert!(is_valid_blur_mode("smoky"));
    assert!(!is_valid_blur_mode("SMOKY"));
}
