// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! CLI planning and execution for `tuxixi update`: flag parsing into
//! an [`UpdatePlan`] and the run loop that dispatches to the theme
//! sync and / or the binary self-update.

use super::fetch::CurlFetcher;
use super::manifest::UPDATE_USAGE;
use super::self_update::{
    apply_self_update, check_self, cleanup_work_dir, fresh_work_dir, SelfCheck,
};
use super::themes::update_themes;

/// What `tuxixi update ...` should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpdatePlan {
    pub self_update: bool,
    pub themes: bool,
    pub check_only: bool,
    pub help: bool,
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
                SelfCheck::UpToDate => {
                    "SELF: already up to date (binary hash matches the latest release).".to_string()
                }
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
