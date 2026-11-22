//! Port of `TuxixiTerminal.get_dynamic_title` (terminal.py): turns the raw
//! VTE window title / current directory into a short, readable tab title.
//!
//! Pure functions so the tricky string logic can be tested exhaustively.

/// Extract the shell-visible home directory hint: titles like
/// `user@host: ~/Projects` are reduced to the right-hand side.
fn strip_user_host_prefix(raw: &str) -> &str {
    if let Some(idx) = raw.find(':') {
        let before = &raw[..idx];
        if before.contains('@') {
            return raw[idx + 1..].trim();
        }
    }
    raw
}

/// `~`-abbreviate an absolute path that lives under `home`.
fn abbreviate_home(raw: &str, home: &str) -> String {
    // Correct boundary handling: `/home/zebra` must NOT match home=/home/z
    // (the Python version had this prefix bug; fixed here).
    if raw == home {
        return "~".to_string();
    }
    let with_sep = format!("{}/", home);
    if let Some(rest) = raw.strip_prefix(&with_sep) {
        return format!("~/{rest}");
    }
    raw.to_string()
}

fn basename_of(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    match trimmed.rsplit('/').next() {
        Some(base) if !base.is_empty() => base.to_string(),
        _ => "/".to_string(),
    }
}

/// Port of `get_dynamic_title`.
///
/// * `raw_title`: VTE window title (may be empty).
/// * `cwd`: current directory from the VTE OSC 7 URI (may be None).
/// * `home`: the user's home directory.
pub fn dynamic_tab_title(raw_title: &str, cwd: Option<&str>, home: &str) -> String {
    let home = home.trim_end_matches('/');
    let raw = raw_title.trim();

    if !raw.is_empty() {
        let mut raw = strip_user_host_prefix(raw).to_string();
        raw = abbreviate_home(&raw, home);

        if raw == "~" || raw == "/" {
            return raw;
        }

        // Path-like titles are reduced to their last component.
        if raw.contains('/') && !raw.starts_with('-') {
            let base = basename_of(&raw);
            if base != "/" {
                return base;
            }
        }
        return raw;
    }

    // No window title: derive the name from the current directory URI.
    if let Some(cwd) = cwd {
        let cwd_norm = cwd.trim_end_matches('/');
        let cwd_norm = if cwd_norm.is_empty() { "/" } else { cwd_norm };
        if cwd_norm == home {
            return "~".to_string();
        }
        let base = basename_of(cwd_norm);
        if base != "/" {
            return base;
        }
        return "/".to_string();
    }

    "Terminal".to_string()
}

/// Port of the tab display title formatting from `ui.py`:
/// `1: ~`, `2: Projects`, or `Terminal N` when there is no dynamic title.
pub fn tab_display_title(index: usize, dyn_title: Option<&str>) -> String {
    match dyn_title {
        Some(t) if !t.is_empty() => format!("{index}: {t}"),
        _ => format!("Terminal {index}"),
    }
}

/// Port of the window title formatting: `TUXIXI — <title>` or `TUXIXI`.
pub fn window_title(dyn_title: Option<&str>) -> String {
    match dyn_title {
        Some(t) if !t.is_empty() => format!("TUXIXI — {t}"),
        _ => "TUXIXI".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/z";

    #[test]
    fn empty_title_with_cwd_uses_directory() {
        assert_eq!(dynamic_tab_title("", Some("/home/z"), HOME), "~");
        assert_eq!(dynamic_tab_title("", Some("/home/z/Projeler"), HOME), "Projeler");
        assert_eq!(dynamic_tab_title("", Some("/home/z/Projeler/"), HOME), "Projeler");
        assert_eq!(dynamic_tab_title("", Some("/etc"), HOME), "etc");
    }

    #[test]
    fn root_directory_names_as_root() {
        assert_eq!(dynamic_tab_title("", Some("/"), HOME), "/");
        assert_eq!(dynamic_tab_title("/", None, HOME), "/");
    }

    #[test]
    fn no_title_no_cwd_is_terminal() {
        assert_eq!(dynamic_tab_title("", None, HOME), "Terminal");
        assert_eq!(dynamic_tab_title("   ", None, HOME), "Terminal");
    }

    #[test]
    fn user_host_prefix_is_stripped() {
        assert_eq!(
            dynamic_tab_title("z@dev: ~/Projeler", None, HOME),
            "Projeler"
        );
        assert_eq!(
            dynamic_tab_title("z@dev:/var/log", None, HOME),
            "log"
        );
        // No '@' before the colon: NOT a user@host title, so the
        // path rule applies and the basename is used (same as Python).
        assert_eq!(
            dynamic_tab_title("vim: /tmp/file", None, HOME),
            "file"
        );
    }

    #[test]
    fn home_paths_are_abbreviated_then_basenamed() {
        assert_eq!(dynamic_tab_title("/home/z/Projeler/tuxixi", None, HOME), "tuxixi");
        assert_eq!(dynamic_tab_title("~/Projeler", None, HOME), "Projeler");
        assert_eq!(dynamic_tab_title("~", None, HOME), "~");
    }

    #[test]
    fn home_prefix_is_not_confused_with_similar_dirs() {
        // Regression: /home/zebra must not be abbreviated to ~ebra.
        assert_eq!(dynamic_tab_title("/home/zebra", None, HOME), "zebra");
        assert_eq!(dynamic_tab_title("/home/z98/projects", None, HOME), "projects");
    }

    #[test]
    fn plain_program_titles_pass_through() {
        assert_eq!(dynamic_tab_title("htop", None, HOME), "htop");
        assert_eq!(dynamic_tab_title("nano file.txt", None, HOME), "nano file.txt");
    }

    #[test]
    fn dash_titles_are_not_treated_as_paths() {
        assert_eq!(dynamic_tab_title("-bash", None, HOME), "-bash");
    }

    #[test]
    fn trailing_slash_paths_resolve_to_basename() {
        assert_eq!(dynamic_tab_title("/home/z/Projects/foo/", None, HOME), "foo");
    }

    #[test]
    fn tab_display_titles() {
        assert_eq!(tab_display_title(1, Some("~")), "1: ~");
        assert_eq!(tab_display_title(2, Some("Projects")), "2: Projects");
        assert_eq!(tab_display_title(3, None), "Terminal 3");
        assert_eq!(tab_display_title(3, Some("")), "Terminal 3");
    }

    #[test]
    fn window_titles() {
        assert_eq!(window_title(Some("htop")), "TUXIXI — htop");
        assert_eq!(window_title(None), "TUXIXI");
    }

    #[test]
    fn basename_edge_cases() {
        assert_eq!(basename_of("/"), "/");
        assert_eq!(basename_of("foo"), "foo");
        assert_eq!(basename_of("a/b/c/"), "c");
    }
}
