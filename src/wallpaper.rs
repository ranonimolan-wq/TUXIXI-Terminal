// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Port of `themes.py` wallpaper detection (`_clean_path`,
//! `detect_wallpaper_path`): KDE, LXQt/PCManFM, XFCE, GNOME/Cinnamon/MATE
//! (gsettings), feh and nitrogen are probed in the same order.
//!
//! File reading is separated from decision logic so it can be tested
//! against fixture files without a desktop environment.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Port of `_clean_path`: strip quotes, unwrap `file://`, percent-decode,
/// expand `~`, accept plain files, and dig into `.desktop` style dirs for
/// the last sorted image in `contents/images_dark`, `contents/images`.
pub fn clean_path(raw_path: &str, home: &str, base: &Path) -> Option<PathBuf> {
    let raw = raw_path.trim();
    if raw.is_empty() {
        return None;
    }
    let mut path = raw
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string();

    if let Some(rest) = path.strip_prefix("file://") {
        path = rest.to_string();
    }
    path = percent_decode(&path);

    let expanded = if let Some(rest) = path.strip_prefix("~/") {
        base.join(home).join(rest)
    } else if path == "~" {
        base.join(home)
    } else {
        base.join(&path)
    };

    if expanded.is_file() {
        return Some(expanded);
    }

    if expanded.is_dir() {
        for sub in ["contents/images_dark", "contents/images", ""] {
            let dir = if sub.is_empty() {
                expanded.clone()
            } else {
                expanded.join(sub)
            };
            if dir.is_dir() {
                if let Some(last) = last_image_in(&dir) {
                    return Some(last);
                }
            }
        }
    }
    None
}

/// Port of the glob patterns `*.[jp][pn]*g` (jpg, jpeg, png) and `*.webp`.
fn is_image_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".webp")
}

fn last_image_in(dir: &Path) -> Option<PathBuf> {
    let mut images: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_image_file(&name) {
                images.push(entry.path());
            }
        }
    }
    images.sort();
    images.last().cloned()
}

/// Small percent-decoder for `file://` URIs (%20 etc.). Invalid escapes
/// are passed through unchanged.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolution order and parsing rules ported from
/// `detect_wallpaper_path`. `gsettings_runner` is injectable for tests;
/// in production it shells out to `gsettings get <schema> <key>`.
pub fn detect_wallpaper_path<F>(home: &str, base: &Path, gsettings_runner: F) -> Option<PathBuf>
where
    F: Fn(&str, &str) -> Option<String>,
{
    // 1. KDE Plasma
    let kde_cfg = base.join(home).join(".config/plasma-org.kde.plasma.desktop-appletsrc");
    if kde_cfg.is_file() {
        if let Ok(content) = std::fs::read_to_string(&kde_cfg) {
            for candidate in content
                .lines()
                .filter_map(|l| l.strip_prefix("Image="))
                .rev()
            {
                if let Some(resolved) = clean_path(candidate, home, base) {
                    return Some(resolved);
                }
            }
        }
    }

    // 2. LXQt / PCManFM-Qt (glob .config/pcmanfm-qt/*/settings.conf)
    let pcmanfm_glob = base.join(home).join(".config/pcmanfm-qt");
    if pcmanfm_glob.is_dir() {
        let mut cfgs: Vec<PathBuf> = std::fs::read_dir(&pcmanfm_glob)
            .ok()?
            .flatten()
            .map(|e| e.path().join("settings.conf"))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            if let Ok(content) = std::fs::read_to_string(&cfg) {
                for line in content.lines() {
                    if let Some(v) = line.strip_prefix("Wallpaper=") {
                        if let Some(resolved) = clean_path(v, home, base) {
                            return Some(resolved);
                        }
                    }
                }
            }
        }
    }

    // 3. XFCE (last-image / image-path attributes in xfce4-desktop.xml)
    let xfce_cfg = base
        .join(home)
        .join(".config/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml");
    if xfce_cfg.is_file() {
        if let Ok(content) = std::fs::read_to_string(&xfce_cfg) {
            for candidate in extract_xfce_values(&content).into_iter().rev() {
                if let Some(resolved) = clean_path(&candidate, home, base) {
                    return Some(resolved);
                }
            }
        }
    }

    // 4. GNOME / Cinnamon / MATE via gsettings (same schema order)
    let queries = [
        ("org.gnome.desktop.background", "picture-uri-dark"),
        ("org.gnome.desktop.background", "picture-uri"),
        ("org.cinnamon.desktop.background", "picture-uri"),
        ("org.mate.background", "picture-filename"),
    ];
    for (schema, key) in queries {
        if let Some(out) = gsettings_runner(schema, key) {
            let out = out.trim().to_string();
            if !out.is_empty() && out != "''" {
                if let Some(resolved) = clean_path(&out, home, base) {
                    return Some(resolved);
                }
            }
        }
    }

    // 5. feh
    let feh_cfg = base.join(home).join(".fehbg");
    if feh_cfg.is_file() {
        if let Ok(content) = std::fs::read_to_string(&feh_cfg) {
            for candidate in extract_quoted(&content).into_iter().rev() {
                if let Some(resolved) = clean_path(&candidate, home, base) {
                    return Some(resolved);
                }
            }
        }
    }

    // 6. nitrogen
    let nit_cfg = base.join(home).join(".config/nitrogen/bg-saved.cfg");
    if nit_cfg.is_file() {
        if let Ok(content) = std::fs::read_to_string(&nit_cfg) {
            for line in content.lines() {
                if let Some(v) = line.strip_prefix("file=") {
                    if let Some(resolved) = clean_path(v, home, base) {
                        return Some(resolved);
                    }
                }
            }
        }
    }

    None
}

/// Extract `value="..."` from `<property name="last-image" ...>` /
/// `name="image-path"` tags without pulling in a regex engine.
fn extract_xfce_values(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in xml.split('<') {
        if !tag.contains("last-image") && !tag.contains("image-path") {
            continue;
        }
        if let Some(idx) = tag.find("value=\"") {
            let rest = &tag[idx + 7..];
            if let Some(end) = rest.find('"') {
                out.push(rest[..end].to_string());
            }
        }
    }
    out
}

/// Extract all `'...'` quoted strings (used for `.fehbg`).
fn extract_quoted(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            let mut buf = String::new();
            for c2 in chars.by_ref() {
                if c2 == '\'' {
                    out.push(buf);
                    break;
                }
                buf.push(c2);
            }
        }
    }
    out
}

/// Real gsettings runner used by the GUI/binary path.
pub fn real_gsettings_runner(schema: &str, key: &str) -> Option<String> {
    Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuxixi-wp-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn noop_gsettings(_schema: &str, _key: &str) -> Option<String> {
        None
    }

    #[test]
    fn percent_decode_basic_and_invalid() {
        assert_eq!(percent_decode("/home/z/My%20Pictures/wall.png"), "/home/z/My Pictures/wall.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%ZZ"), "%ZZ");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn clean_path_accepts_plain_file_and_file_uri() {
        let base = tempdir("cleanfile");
        let img = base.join("wall.png");
        fs::write(&img, "fake").unwrap();

        assert_eq!(clean_path("wall.png", "/home/z", &base), Some(img.clone()));
        assert_eq!(clean_path("file://wall.png", "/home/z", &base), Some(img.clone()));
        assert_eq!(clean_path("'file:///wall.png'", "/home/z", &base), None);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn clean_path_expands_home_and_quotes() {
        let base = tempdir("cleanhome");
        let home = base.join("myhome");
        fs::create_dir_all(home.join("Pictures")).unwrap();
        let img = home.join("Pictures/wall.jpg");
        fs::write(&img, "fake").unwrap();

        assert_eq!(
            clean_path("~/Pictures/wall.jpg", home.to_str().unwrap(), &base),
            Some(img.clone())
        );
        // quoted variant
        assert_eq!(
            clean_path("'~/Pictures/wall.jpg'", home.to_str().unwrap(), &base),
            Some(img.clone())
        );
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn clean_path_digs_into_desktop_dir_variants() {
        let base = tempdir("deskdir");
        let dir = base.join("wallpaperdir");
        fs::create_dir_all(dir.join("contents/images")).unwrap();
        fs::write(dir.join("contents/images/a.png"), "fake").unwrap();
        fs::write(dir.join("contents/images/b.png"), "fake").unwrap();

        // Last sorted image wins (b.png after a.png).
        assert_eq!(
            clean_path("wallpaperdir", "/home/z", &base),
            Some(dir.join("contents/images/b.png"))
        );
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn kde_config_is_parsed_with_last_image_wins() {
        let base = tempdir("kde");
        let home = base.join("h");
        fs::create_dir_all(home.join(".config")).unwrap();
        fs::write(base.join("real.png"), "fake").unwrap();
        fs::write(home.join(".config/plasma-org.kde.plasma-desktop-appletsrcx"), "").ok();
        let cfg = home.join(".config/plasma-org.kde.plasma.desktop-appletsrc");
        fs::write(
            &cfg,
            "Image=/nonexistent/first.png\nImage=/nonexistent/second.png\nImage=real.png\n",
        )
        .unwrap();

        let wp = detect_wallpaper_path(home.to_str().unwrap(), &base, noop_gsettings).unwrap();
        assert!(wp.ends_with("real.png"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn xfce_xml_is_parsed_without_regex() {
        let base = tempdir("xfce");
        let home = base.join("h");
        fs::create_dir_all(home.join(".config/xfce4/xfconf/xfce-perchannel-xml")).unwrap();
        fs::write(base.join("xfce_wall.png"), "fake").unwrap();
        let cfg = home.join(".config/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml");
        fs::write(
            &cfg,
            r#"<property name="last-image" type="string" value="/nonexistent.png"/>
               <property name="image-path" type="string" value="xfce_wall.png"/>"#,
        )
        .unwrap();

        let wp = detect_wallpaper_path(home.to_str().unwrap(), &base, noop_gsettings).unwrap();
        assert!(wp.ends_with("xfce_wall.png"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn nitrogen_config_is_parsed() {
        let base = tempdir("nit");
        let home = base.join("h");
        fs::create_dir_all(home.join(".config/nitrogen")).unwrap();
        fs::write(base.join("nit_wall.png"), "fake").unwrap();
        let cfg = home.join(".config/nitrogen/bg-saved.cfg");
        fs::write(&cfg, "[xffm]\nfile=nit_wall.png\n").unwrap();

        let wp = detect_wallpaper_path(home.to_str().unwrap(), &base, noop_gsettings).unwrap();
        assert!(wp.ends_with("nit_wall.png"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn feh_quoted_paths_are_extracted() {
        let base = tempdir("feh");
        let home = base.join("h");
        fs::create_dir_all(&home).unwrap();
        fs::write(base.join("feh_wall.png"), "fake").unwrap();
        let cfg = home.join(".fehbg");
        fs::write(&cfg, "feh --bg-scale '/nonexistent.png' --bg-fill 'feh_wall.png'").unwrap();

        let wp = detect_wallpaper_path(home.to_str().unwrap(), &base, noop_gsettings).unwrap();
        assert!(wp.ends_with("feh_wall.png"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn gsettings_runner_is_consulted_with_empty_value_ignored() {
        let base = tempdir("gset");
        let home = base.join("h");
        fs::create_dir_all(&home).unwrap();
        fs::write(base.join("gnome_wall.png"), "fake").unwrap();

        let runner = |schema: &str, key: &str| -> Option<String> {
            if schema == "org.gnome.desktop.background" && key == "picture-uri-dark" {
                Some("'file://gnome_wall.png'".to_string())
            } else {
                None
            }
        };
        let wp = detect_wallpaper_path(home.to_str().unwrap(), &base, runner).unwrap();
        assert!(wp.ends_with("gnome_wall.png"));

        // "''" (empty) values must be ignored.
        let runner_empty = |_s: &str, _k: &str| -> Option<String> { Some("''".to_string()) };
        assert!(detect_wallpaper_path(home.to_str().unwrap(), &base, runner_empty).is_none());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn no_wallpaper_sources_returns_none() {
        let base = tempdir("none");
        let wp = detect_wallpaper_path("/home/none", &base, noop_gsettings);
        assert_eq!(wp, None);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn image_filter_matches_python_globs() {
        assert!(is_image_file("a.jpg"));
        assert!(is_image_file("a.jpeg"));
        assert!(is_image_file("a.png"));
        assert!(is_image_file("a.webp"));
        assert!(is_image_file("A.JPG"));
        assert!(!is_image_file("a.gif"));
        assert!(!is_image_file("a.txt"));
    }
}
