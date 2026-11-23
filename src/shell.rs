//! Port of the shell-handling helpers from `terminal.py`: default shell
//! detection, shell argument building and the PATH cleanup done before
//! spawning a session.

use std::path::Path;

/// Port of `_get_default_shell`: passwd shell first, then `$SHELL`,
/// finally `/bin/sh` — but only when the candidate exists and is
/// executable.
pub fn default_shell(passwd_shell: Option<&str>, env_shell: Option<&str>) -> String {
    if let Some(shell) = passwd_shell {
        if is_executable_file(shell) {
            return shell.to_string();
        }
    }
    if let Some(shell) = env_shell {
        if !shell.is_empty() && is_executable_file(shell) {
            return shell.to_string();
        }
    }
    "/bin/sh".to_string()
}

fn is_executable_file(path: &str) -> bool {
    let p = Path::new(path);
    if let Ok(meta) = std::fs::metadata(p) {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && (meta.permissions().mode() & 0o111) != 0
    } else {
        false
    }
}

/// Port of `_get_shell_argv`:
/// * fish  -> `<shell> -l`
/// * zsh   -> `<shell> -l -i`
/// * bash  -> `<shell> --login -i`
/// * other -> `<shell> -i`
pub fn shell_argv(shell: &str) -> Vec<String> {
    let name = basename(shell);
    match name.as_str() {
        "fish" => vec![shell.to_string(), "-l".to_string()],
        "zsh" => vec![shell.to_string(), "-l".to_string(), "-i".to_string()],
        "bash" => vec![shell.to_string(), "--login".to_string(), "-i".to_string()],
        _ => vec![shell.to_string(), "-i".to_string()],
    }
}

/// Port of the PATH cleanup in `_spawn_shell_pty`: split on `:`,
/// drop empty entries, de-duplicate preserving order, then append the
/// TUXIXI bin dir if it exists and is not already present.
pub fn merge_path(current_path: &str, bin_dir: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in current_path.split(':') {
        if !p.is_empty() && !parts.contains(&p) {
            parts.push(p);
        }
    }
    if Path::new(bin_dir).is_dir() && !parts.contains(&bin_dir) {
        parts.push(bin_dir);
    }
    parts.join(":")
}

/// Extra environment variables injected into every TUXIXI session.
pub fn session_env(bin_dir: &str) -> Vec<(String, String)> {
    vec![
        ("TUXIXI_SESSION".to_string(), "1".to_string()),
        ("TUXIXI_BIN_DIR".to_string(), bin_dir.to_string()),
    ]
}

fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    trimmed.rsplit('/').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_argv_per_shell() {
        assert_eq!(shell_argv("/bin/bash"), vec!["/bin/bash", "--login", "-i"]);
        assert_eq!(shell_argv("/usr/bin/zsh"), vec!["/usr/bin/zsh", "-l", "-i"]);
        assert_eq!(shell_argv("/usr/bin/fish"), vec!["/usr/bin/fish", "-l"]);
        assert_eq!(shell_argv("/bin/ksh"), vec!["/bin/ksh", "-i"]);
        assert_eq!(shell_argv("/bin/dash"), vec!["/bin/dash", "-i"]);
    }

    #[test]
    fn default_shell_prefers_passwd_shell() {
        let passwd = "/bin/bash";
        assert_eq!(default_shell(Some(passwd), Some("/bin/zsh")), "/bin/bash");
    }

    #[test]
    fn default_shell_falls_back_to_env_when_passwd_missing() {
        // /definitely/not/a/real/shell does not exist on the test machine.
        assert_eq!(
            default_shell(Some("/definitely/not/a/real/shell"), Some("/bin/sh")),
            "/bin/sh"
        );
    }

    #[test]
    fn default_shell_ultimate_fallback() {
        assert_eq!(
            default_shell(Some("/definitely/not/a/real/shell"), Some("/also/not/real")),
            "/bin/sh"
        );
    }

    #[test]
    fn default_shell_rejects_non_executable() {
        // /etc/passwd exists but is not executable.
        assert_eq!(
            default_shell(Some("/etc/passwd"), Some("/bin/sh")),
            "/bin/sh"
        );
    }

    #[test]
    fn merge_path_dedups_and_appends() {
        let dir = std::env::temp_dir().join("tuxixi-path-test-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let bin_dir = dir.to_str().unwrap();

        let merged = merge_path("/usr/bin:/usr/local/bin:/usr/bin", bin_dir);
        assert_eq!(merged, format!("/usr/bin:/usr/local/bin:{bin_dir}"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_path_skips_missing_bin_dir() {
        let merged = merge_path("/usr/bin:/bin", "/definitely/not/here");
        assert_eq!(merged, "/usr/bin:/bin");
    }

    #[test]
    fn merge_path_drops_empty_entries() {
        let merged = merge_path("::/usr/bin::", "/nonexistent-dir");
        assert_eq!(merged, "/usr/bin");
    }

    #[test]
    fn session_env_matches_python() {
        let env = session_env("/home/u/.local/opt/tuxixi/bin");
        assert_eq!(env[0], ("TUXIXI_SESSION".into(), "1".into()));
        assert_eq!(
            env[1],
            (
                "TUXIXI_BIN_DIR".into(),
                "/home/u/.local/opt/tuxixi/bin".into()
            )
        );
    }
}
