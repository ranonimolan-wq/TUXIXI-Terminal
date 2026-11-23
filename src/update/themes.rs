// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Theme synchronization (`tuxixi update --theme`): compare the
//! local user themes directory against the upstream manifest and
//! add / update / back up / remove files accordingly. Every write is
//! atomic and content-hash verified.

use std::path::Path;

use super::fetch::{hash_file, sha256_hex, Fetcher};
use super::manifest::{parse_manifest, parse_retired, theme_raw_url, ManifestEntry, MANIFEST_URL};

/// What `update --theme` did (or would do, in check mode).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ThemeUpdateReport {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
    pub backed_up: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<String>,
}

impl ThemeUpdateReport {
    pub fn changed_count(&self) -> usize {
        self.added.len() + self.updated.len() + self.removed.len()
    }

    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        for f in &self.added {
            lines.push(format!("  added   : {f}"));
        }
        for f in &self.updated {
            let note = if self.backed_up.contains(f) {
                " (old copy kept as .bak)"
            } else {
                ""
            };
            lines.push(format!("  updated : {f}{note}"));
        }
        for f in &self.removed {
            lines.push(format!("  removed : {f} (old built-in copy)"));
        }
        for f in &self.failed {
            lines.push(format!("  FAILED  : {f}"));
        }
        if self.changed_count() == 0 && self.failed.is_empty() {
            lines.push(format!(
                "  all {} themes already match the repository (hash identical)",
                self.unchanged.len()
            ));
        }
        lines.join("\n")
    }
}

/// Sync the themes in `user_dir` with the upstream manifest.
///
/// * `dry_run` reports what *would* change without writing anything.
/// * `manifest_override` injects the manifest for tests; `None` fetches
///   it from [`MANIFEST_URL`].
///
/// A local file that differs from upstream is overwritten, but only
/// after its current content is preserved as `<name>.bak` (and only
/// when no `.bak` exists, so a user's own edits are never destroyed).
/// Downloaded bytes are verified against the manifest hash before they
/// are written; a mismatch aborts that file.
pub fn update_themes(
    fetcher: &dyn Fetcher,
    user_dir: &Path,
    dry_run: bool,
    manifest_override: Option<&str>,
) -> Result<ThemeUpdateReport, String> {
    let raw = match manifest_override {
        Some(m) => m.to_string(),
        None => {
            let bytes = fetcher.fetch(MANIFEST_URL)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };
    let entries = parse_manifest(&raw)?;

    if !dry_run {
        std::fs::create_dir_all(user_dir)
            .map_err(|e| format!("cannot create {}: {e}", user_dir.display()))?;
    }

    let mut report = ThemeUpdateReport::default();
    for entry in &entries {
        let local_path = user_dir.join(&entry.file);
        let local_hash = hash_file(&local_path).ok();

        if local_hash.as_deref() == Some(entry.sha256.as_str()) {
            report.unchanged.push(entry.file.clone());
            continue;
        }

        if dry_run {
            // Optimistic: the file exists but differs, or is missing.
            if local_hash.is_some() {
                report.updated.push(entry.file.clone());
            } else {
                report.added.push(entry.file.clone());
            }
            continue;
        }

        let bytes = match fetcher.fetch(&theme_raw_url(&entry.file)) {
            Ok(b) => b,
            Err(e) => {
                report.failed.push(format!("{} ({e})", entry.file));
                continue;
            }
        };
        if sha256_hex(&bytes) != entry.sha256 {
            report.failed.push(format!(
                "{} (downloaded content does not match the manifest hash)",
                entry.file
            ));
            continue;
        }

        if local_path.exists() {
            let backup = local_path.with_extension("bak");
            if !backup.exists() {
                std::fs::copy(&local_path, &backup)
                    .map_err(|e| format!("cannot back up {}: {e}", backup.display()))?;
                report.backed_up.push(entry.file.clone());
            }
        }

        // Nested manifest paths (upstream/...) need their subdirectory.
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }

        write_bytes_atomic(&local_path, &bytes).map_err(|e| {
            report.failed.push(format!("{} ({e})", entry.file));
            format!("cannot write {}: {e}", local_path.display())
        })?;

        if local_hash.is_some() {
            report.updated.push(entry.file.clone());
        } else {
            report.added.push(entry.file.clone());
        }
    }

    cleanup_retired_files(user_dir, &raw, &entries, dry_run, &mut report);
    Ok(report)
}

/// Remove user-directory copies of RETIRED built-in themes. A file is
/// only removed when its content hash exactly matches a retired entry,
/// so user-created or user-modified themes are never touched.
pub(crate) fn cleanup_retired_files(
    user_dir: &Path,
    raw_manifest: &str,
    entries: &[ManifestEntry],
    dry_run: bool,
    report: &mut ThemeUpdateReport,
) {
    let retired = parse_retired(raw_manifest);
    if retired.is_empty() {
        return;
    }
    let listed: std::collections::HashSet<&str> = entries.iter().map(|e| e.file.as_str()).collect();

    for rel_path in collect_local_theme_files(user_dir) {
        if listed.contains(rel_path.as_str()) {
            continue;
        }
        let path = user_dir.join(&rel_path);
        let Ok(hash) = hash_file(&path) else { continue };
        if retired.iter().any(|r| r.sha256 == hash) {
            if !dry_run {
                if std::fs::remove_file(&path).is_err() {
                    continue;
                }
                // Drop empty upstream/ directories left behind.
                if let Some(parent) = path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
            }
            report.removed.push(rel_path);
        }
    }
}

/// Relative paths of all theme files under `user_dir` (depth-limited,
/// sorted; .bak/.new temp files ignored).
pub(crate) fn collect_local_theme_files(user_dir: &Path) -> Vec<String> {
    const SUPPORTED: [&str; 6] = ["json", "itermcolors", "toml", "yml", "yaml", "conf"];
    fn walk(dir: &Path, rel: &str, depth: usize, out: &mut Vec<String>) {
        if depth > 3 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if path.is_dir() {
                walk(&path, &child_rel, depth + 1, out);
            } else if SUPPORTED
                .iter()
                .any(|ext| name.to_ascii_lowercase().ends_with(&format!(".{ext}")))
            {
                out.push(child_rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(user_dir, "", 0, &mut out);
    out
}

pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.new");
    std::fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot move {} into place: {e}", path.display())
    })
}

// ----------------------------------------------------------------------
// Self update
// ----------------------------------------------------------------------
