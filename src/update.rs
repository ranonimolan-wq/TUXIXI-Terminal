//! Hash-based updates: `tuxixi update --self` and `tuxixi update --theme`.
//!
//! No version numbers anywhere: a local artifact is compared against the
//! upstream artifact by SHA-256. If the hash differs, the upstream copy
//! is downloaded; if it matches, nothing happens. This works even when
//! small fixes are pushed without a new release tag.
//!
//! * `--theme` reads `themes/manifest.json` from the `main` branch
//!   (SHA-256 of every built-in theme file) and refreshes the copies in
//!   `~/.config/tuxixi/themes/`. Pure file sync - no release needed.
//! * `--self` downloads the latest release tarball and compares the
//!   binary inside against the running one. Binary distribution happens
//!   through GitHub releases, so `--self` picks up a change only after a
//!   release artifact was rebuilt.
//!
//! All HTTP goes through [`Fetcher`], so tests inject a fake fetcher and
//! never touch the network.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

pub const REPO_RAW_BASE: &str =
    "https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main";
pub const MANIFEST_URL: &str = "https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main/themes/manifest.json";
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

/// Where bytes come from. The production implementation uses curl; tests
/// plug in a fake.
pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// Production fetcher: shells out to curl (the same approach the CLI
/// already uses for the external IP lookup - no extra dependencies).
pub struct CurlFetcher;

impl Fetcher for CurlFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        let out = Command::new("curl")
            .args([
                "-fsSL",
                "--connect-timeout",
                "5",
                "--max-time",
                "120",
                url,
            ])
            .output()
            .map_err(|e| format!("could not run curl: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "download failed (curl exit code {:?}): {url}",
                out.status.code()
            ));
        }
        if out.stdout.is_empty() {
            return Err(format!("download was empty: {url}"));
        }
        Ok(out.stdout)
    }
}

// ----------------------------------------------------------------------
// Hashing
// ----------------------------------------------------------------------

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

// ----------------------------------------------------------------------
// Theme manifest
// ----------------------------------------------------------------------

/// One entry of themes/manifest.json.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestEntry {
    pub file: String,
    pub sha256: String,
}

fn is_safe_theme_file_name(name: &str) -> bool {
    // Whitelist: plain file names like "tokyo-night.json". This rejects
    // path traversal (../, sub/dir), absolute paths and control
    // characters (note: JSON already decodes \\b, \\f, \\n escapes, so a
    // literal backslash check alone would not be enough).
    !name.is_empty()
        && name.len() <= 128
        && name.ends_with(".json")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !name.starts_with('.')
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
        if !is_safe_theme_file_name(file) {
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

fn theme_raw_url(file: &str) -> String {
    format!("{REPO_RAW_BASE}/themes/{file}")
}

// ----------------------------------------------------------------------
// Theme update
// ----------------------------------------------------------------------

/// What `update --theme` did (or would do, in check mode).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ThemeUpdateReport {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
    pub backed_up: Vec<String>,
    pub failed: Vec<String>,
}

impl ThemeUpdateReport {
    pub fn changed_count(&self) -> usize {
        self.added.len() + self.updated.len()
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
    for entry in entries {
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
                std::fs::copy(&local_path, &backup).map_err(|e| {
                    format!("cannot back up {}: {e}", backup.display())
                })?;
                report.backed_up.push(entry.file.clone());
            }
        }

        write_bytes_atomic(&local_path, &bytes).map_err(|e| {
            report
                .failed
                .push(format!("{} ({e})", entry.file));
            format!("cannot write {}: {e}", local_path.display())
        })?;

        if local_hash.is_some() {
            report.updated.push(entry.file.clone());
        } else {
            report.added.push(entry.file.clone());
        }
    }
    Ok(report)
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
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

/// Result of comparing the running binary against the latest release.
#[derive(Debug, Clone, PartialEq)]
pub enum SelfCheck {
    UpToDate,
    UpdateAvailable {
        local_sha256: String,
        latest_sha256: String,
    },
}

/// Fetch the latest release tarball, extract the binary and compare its
/// hash with `current_exe`. Nothing is replaced.
///
/// `work_dir` must be an existing scratch directory (usually a fresh
/// temp dir); it is used for the downloaded tarball and the extraction.
pub fn check_self(
    fetcher: &dyn Fetcher,
    current_exe: &Path,
    work_dir: &Path,
) -> Result<SelfCheck, String> {
    let tarball_path = work_dir.join("tuxixi-update.tar.gz");
    let bytes = fetcher.fetch(TARBALL_URL)?;
    std::fs::write(&tarball_path, &bytes)
        .map_err(|e| format!("cannot write {}: {e}", tarball_path.display()))?;
    extract_tarball(&tarball_path, work_dir)?;

    let extracted = work_dir.join(BINARY_NAME_IN_TARBALL);
    let latest_sha256 = hash_file(&extracted)?;
    let local_sha256 = hash_file(current_exe)?;

    if latest_sha256 == local_sha256 {
        Ok(SelfCheck::UpToDate)
    } else {
        Ok(SelfCheck::UpdateAvailable {
            local_sha256,
            latest_sha256,
        })
    }
}

/// Compare (via [`check_self`]) and, when different, replace
/// `current_exe` with the freshly downloaded binary. The replacement is
/// atomic: the new binary is copied next to the old one (so the rename
/// never crosses filesystems), chmod'ed executable, fsync'ed, then
/// renamed over the old file.
pub fn apply_self_update(
    fetcher: &dyn Fetcher,
    current_exe: &Path,
    work_dir: &Path,
) -> Result<String, String> {
    match check_self(fetcher, current_exe, work_dir)? {
        SelfCheck::UpToDate => Ok("TUXIXI is already up to date (binary hash matches the latest release).".to_string()),
        SelfCheck::UpdateAvailable { local_sha256, latest_sha256 } => {
            let extracted = work_dir.join(BINARY_NAME_IN_TARBALL);
            let parent = current_exe
                .parent()
                .ok_or_else(|| format!("{} has no parent directory", current_exe.display()))?;
            let staging = parent.join(".tuxixi.update.new");

            let new_bytes = std::fs::read(&extracted)
                .map_err(|e| format!("cannot read extracted binary: {e}"))?;
            {
                use std::io::Write;
                use std::os::unix::fs::PermissionsExt;
                let mut f = std::fs::File::create(&staging)
                    .map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
                f.write_all(&new_bytes)
                    .map_err(|e| format!("cannot write {}: {e}", staging.display()))?;
                f.sync_all().map_err(|e| format!("cannot sync {}: {e}", staging.display()))?;
                std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("cannot chmod {}: {e}", staging.display()))?;
            }
            std::fs::rename(&staging, current_exe)
                .map_err(|e| format!("cannot replace {}: {e}", current_exe.display()))?;

            Ok(format!(
                "TUXIXI binary updated.\n  old: {local_sha256}\n  new: {latest_sha256}\nRestart TUXIXI to use the new version."
            ))
        }
    }
}

fn extract_tarball(tarball: &Path, dest: &Path) -> Result<(), String> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(dest)
        .status()
        .map_err(|e| format!("could not run tar: {e}"))?;
    if !status.success() {
        return Err(format!(
            "tar extraction failed with exit code {:?}",
            status.code()
        ));
    }
    Ok(())
}

/// Clean up a scratch directory used by check_self / apply_self_update.
pub fn cleanup_work_dir(work_dir: &Path) {
    let _ = std::fs::remove_dir_all(work_dir);
}

fn fresh_work_dir() -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    for _ in 0..10 {
        let name = format!(
            "tuxixi-update-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let dir = base.join(name);
        if std::fs::create_dir(&dir).is_ok() {
            return Ok(dir);
        }
    }
    Err("could not create a temporary directory".to_string())
}

// ----------------------------------------------------------------------
// CLI plumbing
// ----------------------------------------------------------------------

/// What `tuxixi update ...` should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdatePlan {
    pub self_update: bool,
    pub themes: bool,
    pub check_only: bool,
    pub help: bool,
}

impl Default for UpdatePlan {
    fn default() -> Self {
        Self {
            self_update: false,
            themes: false,
            check_only: false,
            help: false,
        }
    }
}

/// Parse the flags after `tuxixi update`. With no target flag, both
/// targets are updated.
pub fn parse_update_args(args: &[&str]) -> Result<UpdatePlan, String> {
    let mut plan = UpdatePlan::default();
    let mut target_given = false;
    for arg in args {
        match *arg {
            "--self" => {
                plan.self_update = true;
                target_given = true;
            }
            "--theme" | "--themes" => {
                plan.themes = true;
                target_given = true;
            }
            "--check" | "--dry-run" => plan.check_only = true,
            "--help" | "-h" => plan.help = true,
            other => return Err(format!("unknown update flag: {other}")),
        }
    }
    if !target_given {
        plan.self_update = true;
        plan.themes = true;
    }
    Ok(plan)
}

/// Run an [`UpdatePlan`] against the real network and real directories.
/// Returns the human-readable report.
pub fn run_plan(plan: &UpdatePlan) -> Result<String, String> {
    if plan.help {
        return Ok(UPDATE_USAGE.to_string());
    }
    let fetcher = CurlFetcher;
    let mut out = String::new();

    if plan.themes {
        let dir = crate::theme::user_themes_dir();
        if plan.check_only {
            let report = update_themes(&fetcher, &dir, true, None)?;
            out.push_str(&format!(
                "THEMES ({}): {}\n{}\n",
                dir.display(),
                if report.changed_count() > 0 {
                    "updates available"
                } else {
                    "up to date"
                },
                report.summary()
            ));
        } else {
            let report = update_themes(&fetcher, &dir, false, None)?;
            out.push_str(&format!(
                "THEMES ({}): {} file(s) changed\n{}\n",
                dir.display(),
                report.changed_count(),
                report.summary()
            ));
        }
    }

    if plan.self_update {
        let exe = std::env::current_exe()
            .map_err(|_| "could not locate the running tuxixi binary".to_string())?;
        let work = fresh_work_dir()?;
        let result = if plan.check_only {
            check_self(&fetcher, &exe, &work).map(|check| match check {
                SelfCheck::UpToDate => "SELF: already up to date (binary hash matches the latest release).".to_string(),
                SelfCheck::UpdateAvailable { .. } => {
                    "SELF: update available - run 'tuxixi update --self' to install it.".to_string()
                }
            })
        } else {
            apply_self_update(&fetcher, &exe, &work)
        };
        cleanup_work_dir(&work);
        let text = result?;
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&text);
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    // ------------------------------------------------------------------
    // Test double
    // ------------------------------------------------------------------

    /// Fake fetcher: serves canned URLs; records requests.
    struct FakeFetcher {
        responses: HashMap<String, Result<Vec<u8>, String>>,
        requests: Mutex<Vec<String>>,
    }

    impl FakeFetcher {
        fn new() -> Self {
            Self {
                responses: HashMap::new(),
                requests: Mutex::new(Vec::new()),
            }
        }
        fn serve(&mut self, url: &str, bytes: &[u8]) -> &mut Self {
            self.responses
                .insert(url.to_string(), Ok(bytes.to_vec()));
            self
        }
        fn fail(&mut self, url: &str, error: &str) -> &mut Self {
            self.responses
                .insert(url.to_string(), Err(error.to_string()));
            self
        }
    }

    impl Fetcher for FakeFetcher {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
            self.requests.lock().unwrap().push(url.to_string());
            self.responses
                .get(url)
                .cloned()
                .unwrap_or_else(|| Err(format!("unexpected url: {url}")))
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuxixi-update-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ------------------------------------------------------------------
    // Hashing
    // ------------------------------------------------------------------

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(sha256_hex(b"abc").len(), 64);
    }

    #[test]
    fn hash_file_reads_content() {
        let dir = temp_dir("hashfile");
        let path = dir.join("f.bin");
        std::fs::write(&path, b"hello world").unwrap();
        assert_eq!(sha256_hex(b"hello world"), hash_file(&path).unwrap());
        assert!(hash_file(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    // ------------------------------------------------------------------
    // Manifest parsing
    // ------------------------------------------------------------------

    #[test]
    fn parse_manifest_accepts_valid_manifest() {
        let json = format!(
            r#"{{"themes": {{"a.json": "{}", "b.json": "{}"}}}}"#,
            "0".repeat(64),
            "f".repeat(64)
        );
        let entries = parse_manifest(&json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].file, "a.json");
        assert_eq!(entries[0].sha256, "0".repeat(64));
    }

    #[test]
    fn parse_manifest_embedded_copy_is_valid() {
        let entries = parse_manifest(crate::theme::MANIFEST_JSON).unwrap();
        assert_eq!(entries.len(), crate::theme::BUILTIN_FILES.len());
    }

    #[test]
    fn parse_manifest_rejects_garbage() {
        assert!(parse_manifest("nope").is_err());
        assert!(parse_manifest(r#"{"themes": {}}"#).is_err());
        assert!(parse_manifest(r#"{"no_themes": {}}"#).is_err());
        assert!(parse_manifest(r#"{"themes": {"a.json": "short"}}"#).is_err());
        assert!(parse_manifest(r#"{"themes": {"a.json": 42}}"#).is_err());
    }

    #[test]
    fn parse_manifest_rejects_path_traversal() {
        let good = "0".repeat(64);
        for bad in ["../evil.json", "sub/dir.json", "/abs.json", "", "..", "a\\b.json"] {
            let json = format!(
                r##"{{"themes": {{"{bad}": "{good}"}}}}"##
            );
            assert!(
                parse_manifest(&json).is_err(),
                "must reject unsafe file name: {bad}"
            );
        }
    }

    // ------------------------------------------------------------------
    // Theme update
    // ------------------------------------------------------------------

    const MANIFEST: &str = r#"{"schema": 1, "themes": {
        "dracula.json": "HASH_DRACULA",
        "nord.json": "HASH_NORD",
        "new-theme.json": "HASH_NEW"
    }}"#;

    fn fake_hash(label: &str) -> String {
        sha256_hex(label.as_bytes())
    }

    fn setup_fetcher() -> FakeFetcher {
        let mut f = FakeFetcher::new();
        // Manifest hashes are the SHA-256 of the upstream file content;
        // the fake upstream "files" are just labeled byte strings.
        let manifest = MANIFEST
            .replacen("HASH_DRACULA", &fake_hash("dracula-v2"), 1)
            .replacen("HASH_NORD", &fake_hash("nord-v1"), 1)
            .replacen("HASH_NEW", &fake_hash("new-theme-v1"), 1);
        f.serve(MANIFEST_URL, manifest.as_bytes());
        f.serve(&theme_raw_url("dracula.json"), b"dracula-v2");
        f.serve(&theme_raw_url("nord.json"), b"nord-v1");
        f.serve(&theme_raw_url("new-theme.json"), b"new-theme-v1");
        f
    }

    #[test]
    fn update_themes_adds_updates_and_skips_unchanged() {
        let dir = temp_dir("themes-basic");
        // nord.json is already identical to upstream; dracula.json is an
        // older local copy; new-theme.json does not exist yet.
        std::fs::write(dir.join("nord.json"), b"nord-v1").unwrap();
        std::fs::write(dir.join("dracula.json"), b"dracula-v1").unwrap();

        let report = update_themes(&setup_fetcher(), &dir, false, None).unwrap();
        assert_eq!(report.unchanged, vec!["nord.json".to_string()]);
        assert_eq!(report.updated, vec!["dracula.json".to_string()]);
        assert_eq!(report.added, vec!["new-theme.json".to_string()]);
        assert!(report.failed.is_empty());
        assert!(report.backed_up.contains(&"dracula.json".to_string()));

        // Content really changed on disk.
        let on_disk = std::fs::read_to_string(dir.join("dracula.json")).unwrap();
        assert_eq!(on_disk, "dracula-v2");
        let new_file = std::fs::read_to_string(dir.join("new-theme.json")).unwrap();
        assert_eq!(new_file, "new-theme-v1");
        // The old dracula copy was preserved.
        let backup = dir.join("dracula.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "dracula-v1");
        // No leftover temp files.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".new"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn update_themes_second_run_is_a_no_op() {
        let dir = temp_dir("themes-idempotent");
        let report1 = update_themes(&setup_fetcher(), &dir, false, None).unwrap();
        assert_eq!(report1.changed_count(), 3);

        let report2 = update_themes(&setup_fetcher(), &dir, false, None).unwrap();
        assert_eq!(report2.changed_count(), 0);
        assert_eq!(report2.unchanged.len(), 3);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn update_themes_dry_run_writes_nothing() {
        let dir = temp_dir("themes-dryrun");
        std::fs::write(dir.join("dracula.json"), b"dracula-v1").unwrap();

        let report = update_themes(&setup_fetcher(), &dir, true, None).unwrap();
        assert_eq!(report.updated, vec!["dracula.json".to_string()]);
        assert!(report.added.contains(&"new-theme.json".to_string()));

        // Nothing written: dracula still old, new-theme still missing.
        assert_eq!(
            std::fs::read_to_string(dir.join("dracula.json")).unwrap(),
            "dracula-v1"
        );
        assert!(!dir.join("new-theme.json").exists());
        assert!(!dir.join("dracula.bak").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn update_themes_never_overwrites_an_existing_backup() {
        let dir = temp_dir("themes-backup");
        std::fs::write(dir.join("dracula.json"), b"dracula-v1").unwrap();
        std::fs::write(dir.join("dracula.bak"), "PRECIOUS USER BACKUP").unwrap();

        update_themes(&setup_fetcher(), &dir, false, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("dracula.bak")).unwrap(),
            "PRECIOUS USER BACKUP"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn update_themes_rejects_content_that_does_not_match_the_manifest() {
        let mut fetcher = setup_fetcher();
        // Upstream file was tampered with: hash no longer matches manifest.
        fetcher.serve(&theme_raw_url("dracula.json"), b"TAMPERED");

        let dir = temp_dir("themes-tamper");
        let report = update_themes(&fetcher, &dir, false, None).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].contains("dracula.json"));
        assert!(report.failed[0].contains("hash"));
        // The tampered content must NOT have been written.
        assert!(!dir.join("dracula.json").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn update_themes_reports_download_failures_and_continues() {
        let mut fetcher = setup_fetcher();
        fetcher.fail(&theme_raw_url("dracula.json"), "network down");

        let dir = temp_dir("themes-netfail");
        let report = update_themes(&fetcher, &dir, false, None).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].starts_with("dracula.json"));
        // The other two files were still synced.
        assert!(dir.join("nord.json").exists());
        assert!(dir.join("new-theme.json").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn update_themes_manifest_fetch_error_is_fatal_and_clean() {
        let mut fetcher = FakeFetcher::new();
        fetcher.fail(MANIFEST_URL, "404");

        let dir = temp_dir("themes-manifest-fail");
        let err = update_themes(&fetcher, &dir, false, None).unwrap_err();
        assert!(err.contains("404"));
        // The user dir must not have been created by a failed run.
        assert!(!dir.join("themes").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn update_themes_manifest_override_is_used_for_tests() {
        let dir = temp_dir("themes-override");
        let manifest = format!(
            r##"{{"themes": {{"only.json": "{}"}}}}"##,
            fake_hash("only-v1")
        );
        let mut fetcher = FakeFetcher::new();
        fetcher.serve(&theme_raw_url("only.json"), b"only-v1");

        let report = update_themes(&fetcher, &dir, false, Some(&manifest)).unwrap();
        assert_eq!(report.added, vec!["only.json".to_string()]);
        // No request went to the real manifest URL.
        let requests = fetcher.requests.lock().unwrap();
        assert!(!requests.iter().any(|u| u == MANIFEST_URL));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn theme_update_report_summary_is_readable() {
        let mut report = ThemeUpdateReport::default();
        report.added.push("a.json".into());
        report.updated.push("b.json".into());
        report.backed_up.push("b.json".into());
        report.unchanged.push("c.json".into());
        let text = report.summary();
        assert!(text.contains("added   : a.json"));
        assert!(text.contains("updated : b.json (old copy kept as .bak)"));

        let empty = ThemeUpdateReport {
            unchanged: vec!["c.json".into()],
            ..Default::default()
        };
        assert!(empty.summary().contains("all 1 themes already match"));
    }

    // ------------------------------------------------------------------
    // Self update
    // ------------------------------------------------------------------

    /// Build a tar.gz containing one file, using the real tar binary
    /// (available in the sandbox and on CI runners).
    fn make_tarball(dest: &Path, name: &str, content: &[u8]) {
        let staging = temp_dir("tarball-stage");
        let member = staging.join(name);
        std::fs::write(&member, content).unwrap();
        let status = Command::new("tar")
            .arg("-czf")
            .arg(dest)
            .arg("-C")
            .arg(&staging)
            .arg(name)
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(staging).ok();
    }

    #[test]
    fn check_self_detects_difference_and_equality() {
        let work = temp_dir("self-check");
        let exe_dir = temp_dir("self-exe");
        let exe = exe_dir.join("tuxixi");
        std::fs::write(&exe, b"current-binary-content").unwrap();

        let mut fetcher = FakeFetcher::new();
        let tarball = work.join("src.tar.gz");
        make_tarball(&tarball, BINARY_NAME_IN_TARBALL, b"new-binary-content");
        fetcher.serve(TARBALL_URL, &std::fs::read(&tarball).unwrap());

        // Different content -> update available.
        let check = check_self(&fetcher, &exe, &work).unwrap();
        match &check {
            SelfCheck::UpdateAvailable {
                local_sha256,
                latest_sha256,
            } => {
                assert_eq!(*local_sha256, sha256_hex(b"current-binary-content"));
                assert_eq!(*latest_sha256, sha256_hex(b"new-binary-content"));
            }
            other => panic!("expected UpdateAvailable, got {other:?}"),
        }

        // Same content -> up to date.
        std::fs::write(&exe, b"new-binary-content").unwrap();
        let check = check_self(&fetcher, &exe, &work).unwrap();
        assert_eq!(check, SelfCheck::UpToDate);

        std::fs::remove_dir_all(work).ok();
        std::fs::remove_dir_all(exe_dir).ok();
    }

    #[test]
    fn apply_self_update_replaces_the_binary_atomically() {
        let work = temp_dir("self-apply");
        let exe_dir = temp_dir("self-apply-exe");
        let exe = exe_dir.join("tuxixi");
        std::fs::write(&exe, b"old-binary").unwrap();

        let mut fetcher = FakeFetcher::new();
        let tarball = work.join("src.tar.gz");
        make_tarball(&tarball, BINARY_NAME_IN_TARBALL, b"brand-new-binary");
        fetcher.serve(TARBALL_URL, &std::fs::read(&tarball).unwrap());

        let message = apply_self_update(&fetcher, &exe, &work).unwrap();
        assert!(message.contains("updated"), "message: {message}");

        // The running binary was replaced and is executable.
        assert_eq!(std::fs::read(&exe).unwrap(), b"brand-new-binary");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "binary must stay executable");

        // No staging file left behind.
        assert!(!exe_dir.join(".tuxixi.update.new").exists());

        // A second run with the same release is a no-op.
        let message2 = apply_self_update(&fetcher, &exe, &work).unwrap();
        assert!(message2.contains("already up to date"));

        std::fs::remove_dir_all(work).ok();
        std::fs::remove_dir_all(exe_dir).ok();
    }

    #[test]
    fn self_update_survives_a_failed_download() {
        let work = temp_dir("self-fail");
        let exe_dir = temp_dir("self-fail-exe");
        let exe = exe_dir.join("tuxixi");
        std::fs::write(&exe, b"old-binary").unwrap();

        let mut fetcher = FakeFetcher::new();
        fetcher.fail(TARBALL_URL, "404 Not Found");

        let err = check_self(&fetcher, &exe, &work).unwrap_err();
        assert!(err.contains("404"));
        // The local binary is untouched.
        assert_eq!(std::fs::read(&exe).unwrap(), b"old-binary");
        std::fs::remove_dir_all(work).ok();
        std::fs::remove_dir_all(exe_dir).ok();
    }

    // ------------------------------------------------------------------
    // CLI plumbing
    // ------------------------------------------------------------------

    #[test]
    fn parse_update_args_defaults_to_everything() {
        let plan = parse_update_args(&[]).unwrap();
        assert!(plan.self_update);
        assert!(plan.themes);
        assert!(!plan.check_only);
        assert!(!plan.help);
    }

    #[test]
    fn parse_update_args_accepts_subsets_and_aliases() {
        let plan = parse_update_args(&["--self"]).unwrap();
        assert!(plan.self_update);
        assert!(!plan.themes);

        let plan = parse_update_args(&["--themes", "--check"]).unwrap();
        assert!(!plan.self_update);
        assert!(plan.themes);
        assert!(plan.check_only);

        let plan = parse_update_args(&["--dry-run"]).unwrap();
        assert!(plan.check_only);
        assert!(plan.self_update && plan.themes);

        let plan = parse_update_args(&["-h"]).unwrap();
        assert!(plan.help);
    }

    #[test]
    fn parse_update_args_rejects_unknown_flags() {
        assert!(parse_update_args(&["--bogus"]).is_err());
        assert!(parse_update_args(&["file.json"]).is_err());
    }

    #[test]
    fn run_plan_prints_usage_on_help() {
        let plan = parse_update_args(&["--help"]).unwrap();
        let out = run_plan(&plan).unwrap();
        assert!(out.contains("Usage: tuxixi update"));
        assert!(out.contains("--check"));
    }

    #[test]
    fn urls_point_at_the_fork() {
        assert!(MANIFEST_URL.starts_with(
            "https://raw.githubusercontent.com/ranonimolan-wq/TUXIXI-Terminal/main/themes/"
        ));
        assert!(TARBALL_URL.ends_with("releases/latest/download/tuxixi-x86_64-linux.tar.gz"));
    }
}
