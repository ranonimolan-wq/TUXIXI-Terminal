// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Built-in theme data: the upstream theme files embedded into the
//! binary at compile time plus the update manifest used by
//! `tuxixi update --theme` (content-hash based, no versions).

pub const DEFAULT_THEME: &str = "TokyoNight";

/// The built-in theme files embedded into the binary (manifest-relative
/// path, content). The manifest-relative path is also the key used by
/// `tuxixi update --theme`. The order defines the theme cycling
/// sequence (Ctrl+Shift+T).
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    (
        "upstream/Atom-One-Dark.itermcolors",
        include_str!("../../themes/upstream/Atom-One-Dark.itermcolors"),
    ),
    (
        "upstream/Atom-One-Light.itermcolors",
        include_str!("../../themes/upstream/Atom-One-Light.itermcolors"),
    ),
    (
        "upstream/Ayu-Mirage.itermcolors",
        include_str!("../../themes/upstream/Ayu-Mirage.itermcolors"),
    ),
    (
        "upstream/Ayu.itermcolors",
        include_str!("../../themes/upstream/Ayu.itermcolors"),
    ),
    (
        "upstream/Catppuccin-Frappe.itermcolors",
        include_str!("../../themes/upstream/Catppuccin-Frappe.itermcolors"),
    ),
    (
        "upstream/Catppuccin-Latte.itermcolors",
        include_str!("../../themes/upstream/Catppuccin-Latte.itermcolors"),
    ),
    (
        "upstream/Catppuccin-Macchiato.itermcolors",
        include_str!("../../themes/upstream/Catppuccin-Macchiato.itermcolors"),
    ),
    (
        "upstream/Catppuccin-Mocha.itermcolors",
        include_str!("../../themes/upstream/Catppuccin-Mocha.itermcolors"),
    ),
    (
        "upstream/Cyberpunk.itermcolors",
        include_str!("../../themes/upstream/Cyberpunk.itermcolors"),
    ),
    (
        "upstream/Dracula.itermcolors",
        include_str!("../../themes/upstream/Dracula.itermcolors"),
    ),
    (
        "upstream/Everforest-Dark-Med.itermcolors",
        include_str!("../../themes/upstream/Everforest-Dark-Med.itermcolors"),
    ),
    (
        "upstream/Flexoki-Dark.itermcolors",
        include_str!("../../themes/upstream/Flexoki-Dark.itermcolors"),
    ),
    (
        "upstream/Flexoki-Light.itermcolors",
        include_str!("../../themes/upstream/Flexoki-Light.itermcolors"),
    ),
    (
        "upstream/GitHub-Dark.itermcolors",
        include_str!("../../themes/upstream/GitHub-Dark.itermcolors"),
    ),
    (
        "upstream/Gruvbox-Dark.itermcolors",
        include_str!("../../themes/upstream/Gruvbox-Dark.itermcolors"),
    ),
    (
        "upstream/Gruvbox-Light.itermcolors",
        include_str!("../../themes/upstream/Gruvbox-Light.itermcolors"),
    ),
    (
        "upstream/Kanagawa-Wave.itermcolors",
        include_str!("../../themes/upstream/Kanagawa-Wave.itermcolors"),
    ),
    (
        "upstream/Monokai-Classic.itermcolors",
        include_str!("../../themes/upstream/Monokai-Classic.itermcolors"),
    ),
    (
        "upstream/Night-Owl.itermcolors",
        include_str!("../../themes/upstream/Night-Owl.itermcolors"),
    ),
    (
        "upstream/Nord.itermcolors",
        include_str!("../../themes/upstream/Nord.itermcolors"),
    ),
    (
        "upstream/One-Half-Dark.itermcolors",
        include_str!("../../themes/upstream/One-Half-Dark.itermcolors"),
    ),
    (
        "upstream/One-Half-Light.itermcolors",
        include_str!("../../themes/upstream/One-Half-Light.itermcolors"),
    ),
    (
        "upstream/Rose-Pine-Dawn.itermcolors",
        include_str!("../../themes/upstream/Rose-Pine-Dawn.itermcolors"),
    ),
    (
        "upstream/Rose-Pine-Moon.itermcolors",
        include_str!("../../themes/upstream/Rose-Pine-Moon.itermcolors"),
    ),
    (
        "upstream/Rose-Pine.itermcolors",
        include_str!("../../themes/upstream/Rose-Pine.itermcolors"),
    ),
    (
        "upstream/Synthwave.itermcolors",
        include_str!("../../themes/upstream/Synthwave.itermcolors"),
    ),
    (
        "upstream/TokyoNight-Day.itermcolors",
        include_str!("../../themes/upstream/TokyoNight-Day.itermcolors"),
    ),
    (
        "upstream/TokyoNight-Moon.itermcolors",
        include_str!("../../themes/upstream/TokyoNight-Moon.itermcolors"),
    ),
    (
        "upstream/TokyoNight-Storm.itermcolors",
        include_str!("../../themes/upstream/TokyoNight-Storm.itermcolors"),
    ),
    (
        "upstream/TokyoNight.itermcolors",
        include_str!("../../themes/upstream/TokyoNight.itermcolors"),
    ),
    (
        "upstream/iTerm2-Solarized-Dark.itermcolors",
        include_str!("../../themes/upstream/iTerm2-Solarized-Dark.itermcolors"),
    ),
    (
        "upstream/iTerm2-Solarized-Light.itermcolors",
        include_str!("../../themes/upstream/iTerm2-Solarized-Light.itermcolors"),
    ),
];

/// The embedded copy of `themes/manifest.json`: SHA-256 of every built-in
/// theme file, used by `tuxixi update --theme` (hash based, no versions).
pub const MANIFEST_JSON: &str = include_str!("../../themes/manifest.json");
