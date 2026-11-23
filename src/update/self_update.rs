// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Binary self-update (`tuxixi update --self`): download the latest
//! release tarball, verify the binary inside by SHA-256 and swap it
//! in atomically. Nothing is replaced on hash mismatch.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::fetch::hash_file;
use super::fetch::Fetcher;
use super::manifest::{BINARY_NAME_IN_TARBALL, TARBALL_URL};

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
        SelfCheck::UpToDate => Ok(
            "TUXIXI is already up to date (binary hash matches the latest release).".to_string(),
        ),
        SelfCheck::UpdateAvailable {
            local_sha256,
            latest_sha256,
        } => {
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
                f.sync_all()
                    .map_err(|e| format!("cannot sync {}: {e}", staging.display()))?;
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

pub(crate) fn extract_tarball(tarball: &Path, dest: &Path) -> Result<(), String> {
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

pub(crate) fn fresh_work_dir() -> Result<PathBuf, String> {
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
