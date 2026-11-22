//! Ports of the tiny shell utilities shipped in the repo root:
//! * `ll`  -> `ls -lah --color=auto --group-directories-first` (replaced
//!            by a self-contained, human-readable listing)
//! * `cls` -> `clear` with an ANSI fallback

use std::path::Path;

/// `ls -h` style human size: powers of 1024, one decimal, e.g.
/// `1023`, `1.0K`, `1.5K`, `2.3M`, `1.2G`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["", "K", "M", "G", "T", "P", "E"];
    if bytes < 1024 {
        return bytes.to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return bytes.to_string();
    }
    if value >= 10.0 {
        format!("{:.0}{}", value, UNITS[unit])
    } else {
        format!("{:.1}{}", value, UNITS[unit])
    }
}

/// ANSI clear-screen string used by the `cls` fallback:
/// `\033[2J` (clear), `\033[3J` (scrollback), `\033[H` (home).
pub fn cls_string() -> String {
    "\x1b[2J\x1b[3J\x1b[H".to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub mode: u32,
    pub owner: String,
}

/// Build the `ll`-style listing of a directory: directories first,
/// everything sorted by name, human readable sizes, `.` entries hidden.
pub fn ll_entries(path: &Path) -> std::io::Result<Vec<ListEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // -a is not in the original `ll` alias
        }
        let meta = entry.metadata()?;
        let (owner, mode) = unix_meta(&meta);
        entries.push(ListEntry {
            name,
            size: meta.len(),
            is_dir: meta.is_dir(),
            is_symlink: entry.file_type()?.is_symlink(),
            mode,
            owner,
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

fn unix_meta(meta: &std::fs::Metadata) -> (String, u32) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mode = meta.permissions().mode() & 0o777;
    let owner = meta
        .uid()
        .to_string();
    (owner, mode)
}

/// Render one entry the way `ls -lah` would (`drwxr-xr-x`, size, name).
pub fn format_entry(entry: &ListEntry) -> String {
    let type_char = if entry.is_dir {
        'd'
    } else if entry.is_symlink {
        'l'
    } else {
        '-'
    };
    let bits = |bit: u32, on: char, off: char| {
        if entry.mode & bit != 0 {
            on
        } else {
            off
        }
    };
    let perms = format!(
        "{}{}{}{}{}{}{}{}{}",
        bits(0o400, 'r', '-'),
        bits(0o200, 'w', '-'),
        bits(0o100, 'x', '-'),
        bits(0o040, 'r', '-'),
        bits(0o020, 'w', '-'),
        bits(0o010, 'x', '-'),
        bits(0o004, 'r', '-'),
        bits(0o002, 'w', '-'),
        bits(0o001, 'x', '-')
    );
    let suffix = if entry.is_dir { "/" } else { "" };
    format!(
        "{}{} {:>6} {}{}",
        type_char,
        perms,
        human_size(entry.size),
        entry.name,
        suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuxixi-ll-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn human_size_matches_ls_h() {
        assert_eq!(human_size(0), "0");
        assert_eq!(human_size(512), "512");
        assert_eq!(human_size(1023), "1023");
        assert_eq!(human_size(1024), "1.0K");
        assert_eq!(human_size(1536), "1.5K");
        assert_eq!(human_size(10 * 1024), "10K");
        assert_eq!(human_size(1024 * 1024), "1.0M");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0G");
    }

    #[test]
    fn cls_string_clears_scrollback_and_homes_cursor() {
        assert_eq!(cls_string(), "\x1b[2J\x1b[3J\x1b[H");
    }

    #[test]
    fn ll_lists_dirs_first_and_hides_dotfiles() {
        let dir = tempdir("order");
        fs::write(dir.join("b_file.txt"), "hello world").unwrap(); // 11 bytes
        fs::write(dir.join("a_file.txt"), "x").unwrap(); // 1 byte
        fs::create_dir(dir.join("z_dir")).unwrap();
        fs::write(dir.join(".hidden"), "secret").unwrap();

        let entries = ll_entries(&dir).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["z_dir", "a_file.txt", "b_file.txt"]);

        assert!(entries[0].is_dir);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn format_entry_renders_permissions_and_human_sizes() {
        let dir_entry = ListEntry {
            name: "src".into(),
            size: 4096,
            is_dir: true,
            is_symlink: false,
            mode: 0o755,
            owner: "1000".into(),
        };
        assert_eq!(format_entry(&dir_entry), "drwxr-xr-x   4.0K src/");

        let file_entry = ListEntry {
            name: "main.rs".into(),
            size: 1024,
            is_dir: false,
            is_symlink: false,
            mode: 0o644,
            owner: "1000".into(),
        };
        assert_eq!(format_entry(&file_entry), "-rw-r--r--   1.0K main.rs");
    }

    #[test]
    fn symlink_rendered_as_l() {
        let link = ListEntry {
            name: "latest".into(),
            size: 7,
            is_dir: false,
            is_symlink: true,
            mode: 0o777,
            owner: "1000".into(),
        };
        let out = format_entry(&link);
        assert!(out.starts_with('l'));
        assert!(out.contains("latest"));
    }
}
