// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Network abstraction and hashing primitives for the updater.
//! All HTTP goes through the [`Fetcher`] trait so tests inject a
//! fake fetcher and never touch the network; content identity is
//! always SHA-256.

use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

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
            .args(["-fsSL", "--connect-timeout", "5", "--max-time", "120", url])
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
