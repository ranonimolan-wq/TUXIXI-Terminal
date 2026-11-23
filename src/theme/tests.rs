// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Unit tests for theme data integrity, parsing, registry merging
//! and color resolution. Compiled only under `cfg(test)` via the
//! parent module's `#[cfg(test)] mod tests;`.

use std::path::{Path, PathBuf};

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
    let theme =
        Theme::from_json(r##"{"name": "X", "background": "#000000", "foreground": "#FFFFFF"}"##)
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
    assert_ne!(
        a.background, b.background,
        "themes must paint different backgrounds"
    );
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
        assert_eq!(
            *entry,
            standard_ansi(i),
            "slot {i} must be the standard base"
        );
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
    assert_eq!(
        resolved[2],
        standard_ansi(2),
        "missing slot 2 keeps the base"
    );
    assert_eq!(
        resolved[15],
        standard_ansi(15),
        "missing slot 15 keeps the base"
    );
}

#[test]
fn resolve_palette_skips_unparsable_entries() {
    let mut t = bare_theme("Garbage");
    t.palette = vec!["#GGGGGG".to_string(), "#0000FF".to_string()];
    let resolved = resolve_palette(&t);
    assert_eq!(resolved.len(), 16);
    assert_eq!(
        resolved[0],
        standard_ansi(0),
        "garbage entry keeps the base"
    );
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
    assert!(
        theme.palette.is_empty(),
        "fixture sanity: no palette in the file"
    );

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
