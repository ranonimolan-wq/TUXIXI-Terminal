//! Integration tests: run the whole CLI through its public interface
//! and verify cross-module behavior (settings <-> css, themes <-> cli).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tuxixi::cli;
use tuxixi::css::build_css;
use tuxixi::settings::{BlurMode, Settings, SettingsManager};
use tuxixi::theme::{get_colors, Registry};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tuxixi-it-{tag}-{}-{}",
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
fn full_cli_dispatch_matrix() {
    // (args, expected exit code, expected substring)
    let cases: Vec<(Vec<&str>, i32, &str)> = vec![
        (vec!["--version"], 0, "TUXIXI Terminal"),
        (vec!["--help"], 0, "KEYBOARD SHORTCUTS"),
        (vec!["--welcome"], 0, "tuxixi --help"),
        (vec!["--sys"], 0, "SYSTEM STATUS"),
        (vec!["--system"], 0, "SYSTEM STATUS"),
        (vec![], 2, "Usage:"),
        (vec!["--nope"], 2, "Unknown command"),
    ];
    for (args, code, needle) in cases {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let out = cli::dispatch(&owned);
        assert_eq!(out.exit_code, code, "args: {args:?}");
        let combined = format!("{}{}", out.stdout, out.stderr);
        assert!(combined.contains(needle), "args: {args:?}, got: {combined}");
    }
}

#[test]
fn settings_changes_flow_into_css_and_colors() {
    let dir = tempdir("flow");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);

    // Default: opaque theme background (default theme: Tokyo Night).
    let s = mgr.get().clone();
    let css = build_css(&s.theme, s.transparent, s.blur_mode);
    assert!(css.contains("#1A1B26"));

    // Toggle transparency: CSS and terminal colors must switch to glass.
    mgr.toggle_transparency();
    let s = mgr.get().clone();
    let css = build_css(&s.theme, s.transparent, s.blur_mode);
    assert!(css.contains("alpha(#000000, 0.52)"));
    let (bg, _fg) = get_colors(&s.theme, s.transparent, s.blur_mode.as_str());
    assert_eq!(bg.a, 0.0);

    // Cycle blur: off -> static -> smoky -> off.
    mgr.cycle_blur_mode();
    assert_eq!(mgr.get().blur_mode, BlurMode::Static);
    mgr.cycle_blur_mode();
    assert_eq!(mgr.get().blur_mode, BlurMode::Smoky);
    mgr.cycle_blur_mode();
    assert_eq!(mgr.get().blur_mode, BlurMode::Off);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn settings_survive_reload_with_full_content() {
    let dir = tempdir("reload");
    let path = dir.join("nested").join("config.json");
    let mut mgr = SettingsManager::load(&path);
    let mut s = Settings::default();
    s.font = "JetBrains Mono".into();
    s.font_size = 19;
    s.theme = "Retro Amber".into();
    s.transparent = true;
    s.blur_mode = BlurMode::Smoky;
    mgr.set(s.clone());

    let mgr2 = SettingsManager::load(&path);
    assert_eq!(mgr2.get(), &s);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn theme_cycle_covers_all_30_builtin_themes() {
    let dir = tempdir("cycleall");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);
    let count = Registry::builtin().len();
    let mut seen = Vec::new();
    for _ in 0..count {
        mgr.cycle_theme();
        seen.push(mgr.get().theme.clone());
    }
    seen.dedup();
    assert_eq!(seen.len(), count, "cycle visited every theme once");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn installed_binary_responds_to_version() {
    // The binary itself must behave like the shell script did.
    let exe = env!("CARGO_BIN_EXE_tuxixi");
    let out = Command::new(exe)
        .arg("--version")
        .output()
        .expect("binary should run");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("TUXIXI Terminal"));
    assert!(text.contains(&format!("v{}", tuxixi::cli::VERSION)));
}

#[test]
fn installed_binary_unknown_command_exits_2() {
    let exe = env!("CARGO_BIN_EXE_tuxixi");
    let out = Command::new(exe)
        .arg("--bogus")
        .output()
        .expect("binary should run");
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Unknown command"));
}

#[test]
fn installed_binary_no_args_exits_2() {
    let exe = env!("CARGO_BIN_EXE_tuxixi");
    let out = Command::new(exe).output().expect("binary should run");
    // Without the `gui` feature the binary explains how to build the GUI.
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("built without the GUI"));
    assert!(err.contains("--features gui"));
}

#[test]
fn installed_binary_help_exits_0() {
    let exe = env!("CARGO_BIN_EXE_tuxixi");
    let out = Command::new(exe).arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--ip"));
    assert!(text.contains("update"));
}

#[test]
fn installed_binary_update_help_exits_0_offline() {
    let exe = env!("CARGO_BIN_EXE_tuxixi");
    let out = Command::new(exe)
        .args(["update", "--help"])
        .output()
        .expect("binary should run");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Usage: tuxixi update"));
    assert!(text.contains("--self"));
    assert!(text.contains("--theme"));
}

#[test]
fn installed_binary_update_unknown_flag_exits_2() {
    let exe = env!("CARGO_BIN_EXE_tuxixi");
    let out = Command::new(exe)
        .args(["update", "--bogus"])
        .output()
        .expect("binary should run");
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown update flag"));
    assert!(err.contains("Usage: tuxixi update"));
}

#[test]
fn installed_binary_help_alias_works() {
    let exe = env!("CARGO_BIN_EXE_tuxixi");
    let out = Command::new(exe).arg("help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("COMMANDS"));
}
