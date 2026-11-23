// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Unit tests for the updater: a fake [`Fetcher`] feeds canned
//! payloads so every code path (manifest parse, theme sync, self
//! check, plan execution) is covered offline.

use std::path::{Path, PathBuf};
use std::process::Command;

use std::collections::HashMap;
use std::sync::Mutex;

use super::*;

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
        self.responses.insert(url.to_string(), Ok(bytes.to_vec()));
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
    for bad in [
        "../evil.json",
        "sub/dir.json",
        "/abs.json",
        "",
        "..",
        "a\\b.json",
        "other/evil.json",
    ] {
        let json = format!(r##"{{"themes": {{"{bad}": "{good}"}}}}"##);
        assert!(
            parse_manifest(&json).is_err(),
            "must reject unsafe file name: {bad}"
        );
    }
    // The vendored upstream prefix IS allowed.
    let ok = format!(r##"{{"themes": {{"upstream/TokyoNight.itermcolors": "{good}"}}}}"##);
    assert!(parse_manifest(&ok).is_ok());
}

// ------------------------------------------------------------------
// Theme update
// ------------------------------------------------------------------

const MANIFEST: &str = r#"{"schema": 1, "themes": {
    "dracula.json": "HASH_DRACULA",
    "nord.json": "HASH_NORD",
    "new-theme.json": "HASH_NEW"
}}"#;

const RETIRED_MANIFEST: &str = r#"{"schema": 1, "themes": {
    "upstream/New.itermcolors": "HASH_NEW"
}, "retired": {
    "legacy-dark.json": "HASH_LEGACY"
}}"#;

#[test]
fn retired_theme_copies_are_removed_but_user_files_kept() {
    let dir = temp_dir("themes-retired");
    // legacy-dark.json is an exact copy of the retired built-in
    // (hash matches) -> removed. user-own.json is the user's own
    // file (not in the manifest, not retired) -> kept.
    std::fs::write(dir.join("legacy-dark.json"), b"LEGACY-CONTENT").unwrap();
    std::fs::write(dir.join("user-own.json"), b"USER-OWN").unwrap();

    let mut f = FakeFetcher::new();
    let manifest = RETIRED_MANIFEST
        .replacen("HASH_NEW", &fake_hash("new-v1"), 1)
        .replacen("HASH_LEGACY", &fake_hash("LEGACY-CONTENT"), 1);
    f.serve(MANIFEST_URL, manifest.as_bytes());
    f.serve(&theme_raw_url("upstream/New.itermcolors"), b"new-v1");

    let report = update_themes(&f, &dir, false, None).unwrap();
    assert_eq!(report.removed, vec!["legacy-dark.json".to_string()]);
    assert!(
        !dir.join("legacy-dark.json").exists(),
        "retired copy deleted"
    );
    assert!(dir.join("user-own.json").exists(), "user file kept");
    assert!(dir.join("upstream/New.itermcolors").exists());
    assert_eq!(report.added, vec!["upstream/New.itermcolors".to_string()]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn modified_retired_theme_files_are_never_removed() {
    let dir = temp_dir("themes-retired-mod");
    // The user edited the old file -> hash no longer matches the
    // retired entry -> must be kept.
    std::fs::write(dir.join("legacy-dark.json"), b"USER EDITED").unwrap();

    let mut f = FakeFetcher::new();
    let manifest = RETIRED_MANIFEST
        .replacen("HASH_NEW", &fake_hash("new-v1"), 1)
        .replacen("HASH_LEGACY", &fake_hash("LEGACY-CONTENT"), 1);
    f.serve(MANIFEST_URL, manifest.as_bytes());
    f.serve(&theme_raw_url("upstream/New.itermcolors"), b"new-v1");

    let report = update_themes(&f, &dir, false, None).unwrap();
    assert!(report.removed.is_empty());
    assert!(dir.join("legacy-dark.json").exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn dry_run_reports_retired_removals_without_deleting() {
    let dir = temp_dir("themes-retired-dry");
    std::fs::write(dir.join("legacy-dark.json"), b"LEGACY-CONTENT").unwrap();

    let mut f = FakeFetcher::new();
    let manifest = RETIRED_MANIFEST
        .replacen("HASH_NEW", &fake_hash("new-v1"), 1)
        .replacen("HASH_LEGACY", &fake_hash("LEGACY-CONTENT"), 1);
    f.serve(MANIFEST_URL, manifest.as_bytes());

    let report = update_themes(&f, &dir, true, None).unwrap();
    assert_eq!(report.removed, vec!["legacy-dark.json".to_string()]);
    assert!(
        dir.join("legacy-dark.json").exists(),
        "dry run must not delete"
    );
    std::fs::remove_dir_all(dir).ok();
}

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
