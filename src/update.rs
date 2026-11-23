// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

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

mod fetch;
mod manifest;
mod plan;
mod self_update;
mod themes;

pub use fetch::{hash_file, sha256_hex, CurlFetcher, Fetcher};
pub use manifest::{
    parse_manifest, ManifestEntry, BINARY_NAME_IN_TARBALL, MANIFEST_URL, REPO_RAW_BASE,
    TARBALL_URL, UPDATE_USAGE,
};
pub use plan::{parse_update_args, run_plan, UpdatePlan};
pub use self_update::{apply_self_update, check_self, cleanup_work_dir, SelfCheck};
pub use themes::{update_themes, ThemeUpdateReport};

// Crate-visible helper re-exported for the offline unit tests.
#[cfg(test)]
pub(crate) use manifest::theme_raw_url;

#[cfg(test)]
mod tests;
