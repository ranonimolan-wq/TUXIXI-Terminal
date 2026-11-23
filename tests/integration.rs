// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Integration tests: run the whole CLI through its public interface
//! and verify cross-module behavior (settings <-> theme pipeline,
//! themes <-> cli, input encoding <-> vt state).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tuxixi::cli;
use tuxixi::input::{self, Key, Modifiers};
use tuxixi::settings::{BlurMode, Settings, SettingsManager};
use tuxixi::theme::{get_colors, Registry};
use tuxixi::vt::Terminal;

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
        let owned: Vec<String> = args.iter().map(std::string::ToString::to_string).collect();
        let out = cli::dispatch(&owned);
        assert_eq!(out.exit_code, code, "args: {args:?}");
        let combined = format!("{}{}", out.stdout, out.stderr);
        assert!(combined.contains(needle), "args: {args:?}, got: {combined}");
    }
}

#[test]
fn settings_changes_flow_into_terminal_colors() {
    let dir = tempdir("flow");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);

    // Default: opaque Tokyo Night background.
    let s = mgr.get().clone();
    let colors = tuxixi::theme::terminal_colors(&s.theme, false, "off");
    assert_eq!(colors.background, tuxixi::theme::Rgba::opaque("#1A1B26"));

    // Glass mode: background goes fully transparent, fg survives.
    let (bg, fg) = get_colors(&s.theme, true, "glass");
    assert_eq!(bg.a, 0.0);
    assert_eq!(fg, colors.foreground);

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
fn vt_grid_renders_theme_foreground_characters() {
    // End-to-end: settings -> theme colors -> VT grid -> input bytes.
    let mut t = Terminal::new(20, 4);
    t.feed(b"TUXIXI \x1b[32mgreen\x1b[0m");
    assert_eq!(t.cell(0, 0).unwrap().c, 'T');
    assert_eq!(t.cell(7, 0).unwrap().fg, tuxixi::vt::Color::Indexed(2));

    // Typing "q" through the input encoder feeds the same grid.
    let bytes = input::encode(Key::Char('q'), Modifiers::default(), false).unwrap();
    assert_eq!(bytes, b"q");
}

#[test]
fn vt_scrollback_is_hard_capped_end_to_end() {
    let mut t = Terminal::new(20, 4);
    t.set_scrollback_cap(25);
    for i in 0..1000 {
        t.feed(format!("line{i}\r\n").as_bytes());
    }
    assert!(t.scrollback_len() <= 25);
    let report = t.memory_report();
    assert!(report.scrollback_rows <= 25);
    assert_eq!(report.live_cells, 20 * 4);
}

#[test]
fn settings_survive_reload_with_full_content() {
    let dir = tempdir("reload");
    let path = dir.join("nested").join("config.json");
    let mut mgr = SettingsManager::load(&path);
    let s = Settings {
        font: "JetBrains Mono".into(),
        font_size: 19,
        theme: "Dracula".into(),
        transparent: true,
        blur_mode: BlurMode::Smoky,
        glass: false,
        glass_opacity: 0.45,
    };
    mgr.set(s.clone());

    let mgr2 = SettingsManager::load(&path);
    assert_eq!(mgr2.get(), &s);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn theme_cycle_covers_every_registry_theme() {
    let dir = tempdir("cycleall");
    let path = dir.join("config.json");
    let mut mgr = SettingsManager::load(&path);
    // cycle_theme walks the global registry (built-ins + user themes).
    let count = Registry::global().len();
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

// Only meaningful for CLI-only builds: with `gui` the binary opens a
// real terminal window (which a headless test host cannot do).
#[cfg(not(feature = "gui"))]
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
