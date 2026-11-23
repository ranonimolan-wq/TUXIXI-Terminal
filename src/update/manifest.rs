// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Upstream manifest constants and parsing for the theme sync:
//! URL bases, strict path-safety whitelists and the JSON schema
//! (`{"themes": [...], "retired": [...]}`).

//! never touch the network.

pub const REPO_RAW_BASE: &str =
    "https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main";
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main/themes/manifest.json";
pub const TARBALL_URL: &str =
    "https://github.com/ranonimolan-wq/TUXIXI-Terminal/releases/latest/download/tuxixi-x86_64-linux.tar.gz";
pub const BINARY_NAME_IN_TARBALL: &str = "tuxixi";

pub const UPDATE_USAGE: &str = "\
Usage: tuxixi update [--self | --theme] [--check]

  tuxixi update            update everything (binary + themes)
  tuxixi update --self     update the tuxixi binary itself
  tuxixi update --theme    update the theme collection
  tuxixi update --check    only report what would change, write nothing

Updates are hash based: files are compared by SHA-256, so new content
is picked up as soon as it lands in the repository, release or not.";

/// One entry of themes/manifest.json.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestEntry {
    pub file: String,
    pub sha256: String,
}

pub(crate) fn is_safe_theme_file_name(name: &str) -> bool {
    // Whitelist: plain file names like "tokyo-night.json" or
    // "TokyoNight-Storm.itermcolors". This rejects path traversal,
    // absolute paths and control characters (note: JSON already decodes
    // \b, \f, \n escapes, so a literal backslash check alone would not
    // be enough).
    !name.is_empty()
        && name.len() <= 128
        && (name.ends_with(".json") || name.ends_with(".itermcolors"))
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !name.starts_with('.')
}

/// Manifest paths may include the "upstream/" directory prefix (the
/// vendored creator files). The only allowed directory prefix is
/// `upstream`; the file part must be a safe theme file name.
pub(crate) fn is_safe_manifest_path(path: &str) -> bool {
    if path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        return false;
    }
    let components: Vec<&str> = path.split('/').collect();
    let Some(file) = components.last().copied() else {
        return false;
    };
    if !is_safe_theme_file_name(file) {
        return false;
    }
    match components.len() {
        1 => true,
        2 => components[0] == "upstream",
        _ => false,
    }
}

/// Parse a manifest.json string into entries. Rejects malformed hashes
/// and unsafe file names (path traversal attempts).
pub fn parse_manifest(json: &str) -> Result<Vec<ManifestEntry>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("manifest is not valid JSON: {e}"))?;
    let themes = value
        .get("themes")
        .and_then(|t| t.as_object())
        .ok_or("manifest has no 'themes' object")?;
    if themes.is_empty() {
        return Err("manifest lists no themes".to_string());
    }
    let mut entries = Vec::new();
    for (file, hash) in themes {
        if !is_safe_manifest_path(file) {
            return Err(format!("manifest contains unsafe file name: {file}"));
        }
        let hash = hash
            .as_str()
            .ok_or_else(|| format!("manifest entry {file} has no sha256 string"))?;
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("manifest entry {file} has a malformed sha256"));
        }
        entries.push(ManifestEntry {
            file: file.clone(),
            sha256: hash.to_ascii_lowercase(),
        });
    }
    Ok(entries)
}

/// Parse the optional "retired" section of a manifest: file hashes of
/// previously shipped built-in themes that are no longer distributed.
/// Copies of these in the user themes directory are safe to delete
/// (identified by hash, never by name alone).
pub(crate) fn parse_retired(json: &str) -> Vec<ManifestEntry> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(retired) = value.get("retired").and_then(|r| r.as_object()) else {
        return Vec::new();
    };
    retired
        .iter()
        .filter_map(|(file, hash)| {
            let hash = hash.as_str()?;
            if is_safe_manifest_path(file) && hash.len() == 64 {
                Some(ManifestEntry {
                    file: file.clone(),
                    sha256: hash.to_ascii_lowercase(),
                })
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn theme_raw_url(file: &str) -> String {
    format!("{REPO_RAW_BASE}/themes/{file}")
}

// ----------------------------------------------------------------------
// Theme update
// ----------------------------------------------------------------------
