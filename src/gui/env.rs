// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Process environment helpers used when spawning the session:
//! inherited `$SHELL`, the passwd shell of the current uid and the
//! directory of the running binary.

pub(crate) fn shell_env() -> Option<String> {
    std::env::var("SHELL").ok()
}

/// Minimal /etc/passwd lookup for the current uid (port of
/// pwd.getpwuid(os.getuid()).pw_shell, same as the GTK build).
pub(crate) fn passwd_shell() -> Option<String> {
    let uid = nix::unistd::getuid();
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 7 && fields[2].parse::<u32>().ok() == Some(uid.as_raw()) {
            return Some(fields[6].to_string());
        }
    }
    None
}

/// The directory that contains the running binary (e.g. ~/.local/bin).
pub(crate) fn bin_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}
