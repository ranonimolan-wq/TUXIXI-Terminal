//! Port of `bin/tuxixi`: the TUXIXI command line interface.
//!
//! All commands write into injectable writers and return an exit code,
//! which makes the whole CLI unit-testable (see the tests below).

use std::io::Write;

use crate::lsutil;
use crate::sysinfo;
use crate::update;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Result of dispatching one CLI invocation.
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl CliOutput {
    fn new() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        }
    }
}

/// Dispatch `tuxixi <args>` exactly like the `case "$1"` block in the
/// original shell script.
pub fn dispatch(args: &[String]) -> CliOutput {
    let mut out = CliOutput::new();
    let first = args.first().map(|s| s.as_str()).unwrap_or("");

    match first {
        "--welcome" => out.stdout.push_str(&sysinfo::format_welcome()),
        "--help" | "help" => out.stdout.push_str(&help_text()),
        "--version" | "version" => out.stdout.push_str(&sysinfo::format_version()),
        "update" => run_update_command(&args[1..], &mut out),
        "ll" => run_ll(&args[1..], &mut out),
        "cls" => {
            // Clear screen: the ANSI sequence clears the visible area,
            // the scrollback and homes the cursor (see lsutil::cls_string).
            out.stdout.push_str(&lsutil::cls_string());
        }
        "--system" | "--sys" => {
            let info = sysinfo::gather_sys();
            out.stdout.push_str(&sysinfo::format_sys(&info));
        }
        "--ip" => {
            let info = gather_ip();
            out.stdout.push_str(&sysinfo::format_ip(&info));
        }
        "" => {
            out.stderr.push_str(&format!(
                "Usage: {LIME}tuxixi --help{RESET}\n",
                LIME = sysinfo::LIME,
                RESET = sysinfo::RESET
            ));
            out.exit_code = 2;
        }
        unknown => {
            out.stderr.push_str(&format!(
                "{RED}Unknown command:{RESET} {unknown}\n",
                RED = sysinfo::RED,
                RESET = sysinfo::RESET
            ));
            out.stderr.push_str(&format!(
                "Run {LIME}\"tuxixi --help\"{RESET} for available commands.\n",
                LIME = sysinfo::LIME,
                RESET = sysinfo::RESET
            ));
            out.exit_code = 2;
        }
    }
    out
}

/// Convenience wrapper used by the real binary: dispatch + print.
pub fn run_and_print(args: &[String]) -> i32 {
    let out = dispatch(args);
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let _ = stdout.write_all(out.stdout.as_bytes());
    let _ = stderr.write_all(out.stderr.as_bytes());
    let _ = stdout.flush();
    let _ = stderr.flush();
    out.exit_code
}

/// Handle `tuxixi update ...`: parse flags, run the plan, report.
fn run_update_command(rest: &[String], out: &mut CliOutput) {
    let flag_strs: Vec<&str> = rest.iter().map(|s| s.as_str()).collect();
    let plan = match update::parse_update_args(&flag_strs) {
        Ok(plan) => plan,
        Err(e) => {
            out.stderr.push_str(&format!(
                "{RED}{e}{RESET}\n\n",
                RED = sysinfo::RED,
                RESET = sysinfo::RESET
            ));
            out.stderr.push_str(update::UPDATE_USAGE);
            out.stderr.push('\n');
            out.exit_code = 2;
            return;
        }
    };
    match update::run_plan(&plan) {
        Ok(report) => {
            if !plan.help {
                out.stdout
                    .push_str(&format!("{PURPLE}\u{25c6} TUXIXI UPDATE{RESET}\n", PURPLE = sysinfo::PURPLE, RESET = sysinfo::RESET));
            }
            out.stdout.push_str(&report);
            out.exit_code = 0;
        }
        Err(e) => {
            out.stderr.push_str(&format!(
                "{RED}Update failed:{RESET} {e}\n",
                RED = sysinfo::RED,
                RESET = sysinfo::RESET
            ));
            out.exit_code = 1;
        }
    }
}

/// Handle `tuxixi ll [path]`: ls -lah style listing of a directory.
fn run_ll(rest: &[String], out: &mut CliOutput) {
    let target = rest.first().map(|s| s.as_str()).unwrap_or(".");
    let path = std::path::Path::new(target);
    match lsutil::ll_entries(path) {
        Ok(entries) => {
            for entry in &entries {
                out.stdout.push_str(&lsutil::format_entry(entry));
                out.stdout.push('\n');
            }
        }
        Err(e) => {
            out.stderr.push_str(&format!(
                "{RED}ll: {target}: {e}{RESET}\n",
                RED = sysinfo::RED,
                RESET = sysinfo::RESET
            ));
            out.exit_code = 1;
        }
    }
}

fn gather_ip() -> sysinfo::IpInfo {
    let (internal, iface) = internal_ip_and_iface();
    let external = if internal.is_some() { external_ip() } else { None };
    sysinfo::IpInfo {
        internal_ip: internal,
        iface,
        external_ip: external,
    }
}

fn internal_ip_and_iface() -> (Option<String>, Option<String>) {
    // Port of `ip -4 route get 1.1.1.1`, falling back to `hostname -I`.
    if let Ok(output) = std::process::Command::new("ip")
        .args(["-4", "route", "get", "1.1.1.1"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let (src, dev) = sysinfo::parse_ip_route(&text);
            if src.is_some() {
                return (src, dev);
            }
        }
    }
    if let Ok(output) = std::process::Command::new("hostname")
        .arg("-I")
        .output()
    {
        if output.status.success() {
            let first = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                .map(|s| s.to_string());
            if let Some(ip) = first {
                return (Some(ip), None);
            }
        }
    }
    (None, None)
}

fn external_ip() -> Option<String> {
    // Same service as the original script: api.ipify.org via curl.
    std::process::Command::new("curl")
        .args(["-fsS", "--connect-timeout", "1", "--max-time", "2", "https://api.ipify.org"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Exact port of `show_help`.
#[allow(non_snake_case)]
pub fn help_text() -> String {
    let B = sysinfo::BOLD;
    let R = sysinfo::RESET;
    let P = sysinfo::PURPLE;
    let C = sysinfo::CYAN;
    let L = sysinfo::LIME;
    let S = sysinfo::SLATE;
    let A = sysinfo::AMBER;

    let mut out = String::new();
    out.push('\n');
    out.push_str(&format!("{B}{P}◆ TUXIXI TERMINAL{R} {S}v{VERSION}{R}\n"));
    out.push_str(&format!("{S}────────────────────────────────────────{R}\n\n"));

    out.push_str(&format!("{B}{C}COMMANDS{R}\n"));
    out.push_str(&format!("  {L}tuxixi --help{R}      {S}Display help{R}\n"));
    out.push_str(&format!("  {L}tuxixi --version{R}   {S}Version information{R}\n"));
    out.push_str(&format!("  {L}tuxixi --system{R}    {S}System information{R}\n"));
    out.push_str(&format!("  {L}tuxixi --sys{R}       {S}System information{R}\n"));
    out.push_str(&format!("  {L}tuxixi --ip{R}        {S}Network information{R}\n\n"));

    out.push_str(&format!("{B}{C}UPDATES{R}\n"));
    out.push_str(&format!("  {L}tuxixi update{R}           {S}Update binary + themes (hash based){R}\n"));
    out.push_str(&format!("  {L}tuxixi update --self{R}    {S}Update the tuxixi binary itself{R}\n"));
    out.push_str(&format!("  {L}tuxixi update --theme{R}   {S}Sync themes from the repository{R}\n"));
    out.push_str(&format!("  {L}tuxixi update --check{R}   {S}Only show what would change{R}\n\n"));

    out.push_str(&format!("{B}{C}THEMES{R}\n"));
    out.push_str(&format!("  {S}30 built-in themes, default: {DEFAULT}{R}\n", DEFAULT = crate::theme::DEFAULT_THEME));
    out.push_str(&format!("  {S}Add your own JSON files to:{R}\n"));
    out.push_str(&format!("  {L}~/.config/tuxixi/themes/*.json{R}   {S}(they override built-ins){R}\n\n"));

    out.push_str(&format!("{B}{C}UTILITIES{R}\n"));
    out.push_str(&format!("  {L}ll{R}                 {S}List directory contents{R}\n"));
    out.push_str(&format!("  {L}cls{R}                {S}Clear terminal{R}\n\n"));

    out.push_str(&format!("{B}{C}KEYBOARD SHORTCUTS{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+C{R}       {S}Copy{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+V{R}       {S}Paste cleanly{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+Enter{R}   {S}New tab{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+W{R}       {S}Close tab{R}\n"));
    out.push_str(&format!("  {A}Alt+Number / Ctrl+Tab{R} {S}Switch between tabs{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+T{R}       {S}Change theme{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+O{R}       {S}Toggle glass mode (transparency){R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+B{R}       {S}Cycle blur (Static / Smoky / Off){R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+={R}       {S}Increase font size{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+-{R}       {S}Decrease font size{R}\n"));
    out.push_str(&format!("  {A}Ctrl+Shift+N{R}       {S}New window{R}\n\n"));

    out.push_str(&format!("{P}◆{R} {S}Built for terminal enthusiasts{R}\n\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_flag_prints_version() {
        let out = dispatch(&["--version".to_string()]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("TUXIXI Terminal"));
        assert!(out.stdout.contains(&format!("v{VERSION}")), "stdout: {}", out.stdout);
        assert!(out.stdout.ends_with('\n'));
    }

    #[test]
    fn sys_and_system_are_aliases() {
        let a = dispatch(&["--system".to_string()]);
        let b = dispatch(&["--sys".to_string()]);
        assert_eq!(a.exit_code, 0);
        assert_eq!(a.stdout, b.stdout);
        assert!(a.stdout.contains("◆ SYSTEM STATUS"));
        assert!(a.stdout.contains("KERNEL"));
        // Live environment: these sections must always be filled in.
        assert!(!a.stdout.contains("Unavailable"));
    }

    #[test]
    fn welcome_prints_banner() {
        let out = dispatch(&["--welcome".to_string()]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("◆ TUXIXI Terminal"));
        assert!(out.stdout.contains("tuxixi --help"));
    }

    #[test]
    fn help_lists_commands_and_shortcuts() {
        let out = dispatch(&["--help".to_string()]);
        assert_eq!(out.exit_code, 0);
        for needle in [
            "COMMANDS",
            "UTILITIES",
            "KEYBOARD SHORTCUTS",
            "tuxixi --version",
            "Ctrl+Shift+Enter",
            "Built for terminal enthusiasts",
        ] {
            assert!(out.stdout.contains(needle), "missing: {needle}");
        }
    }

    #[test]
    fn no_args_gives_usage_and_exit_code_2() {
        let out = dispatch(&[]);
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("Usage:"));
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn unknown_command_gives_error_and_exit_code_2() {
        let out = dispatch(&["--bogus".to_string()]);
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("Unknown command:"));
        assert!(out.stderr.contains("--bogus"));
        assert!(out.stderr.contains("tuxixi --help"));
    }

    #[test]
    fn help_and_version_aliases_work_without_dashes() {
        let help = dispatch(&["help".to_string()]);
        assert_eq!(help.exit_code, 0);
        assert!(help.stdout.contains("COMMANDS"));
        assert!(help.stdout.contains("tuxixi update"));

        let version = dispatch(&["version".to_string()]);
        assert_eq!(version.exit_code, 0);
        assert!(version.stdout.contains("TUXIXI Terminal"));
    }

    #[test]
    fn update_unknown_flag_prints_usage_and_exit_code_2() {
        let out = dispatch(&["update".to_string(), "--bogus".to_string()]);
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("unknown update flag"));
        assert!(out.stderr.contains("unknown update flag: --bogus"));
        assert!(out.stderr.contains("Usage: tuxixi update"));
    }

    #[test]
    fn update_help_prints_usage_without_touching_the_network() {
        let out = dispatch(&["update".to_string(), "--help".to_string()]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: tuxixi update"));
        assert!(out.stdout.contains("--self"));
        assert!(out.stdout.contains("--theme"));
        assert!(out.stdout.contains("--check"));
        assert!(out.stdout.contains("hash"));
        assert!(out.stderr.is_empty());
    }

    #[test]
    fn help_documents_updates_and_themes() {
        let out = dispatch(&["--help".to_string()]);
        assert!(out.stdout.contains("UPDATES"));
        assert!(out.stdout.contains("THEMES"));
        assert!(out.stdout.contains("Tokyo Night"));
        assert!(out.stdout.contains("~/.config/tuxixi/themes"));
    }

    #[test]
    fn ll_lists_a_temp_directory() {
        let dir = std::env::temp_dir().join(format!(
            "tuxixi-ll-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zebra.txt"), "hello world").unwrap();
        std::fs::create_dir_all(dir.join("alpha")).unwrap();
        // Hidden files are not listed (like the original ll alias).
        std::fs::write(dir.join(".hidden"), "x").unwrap();

        let out = dispatch(&["ll".to_string(), dir.to_string_lossy().into_owned()]);
        assert_eq!(out.exit_code, 0);
        let lines: Vec<&str> = out.stdout.lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 entries, got: {}", out.stdout);
        assert!(lines[0].ends_with("alpha/"), "dirs first: {}", lines[0]);
        assert!(lines[0].starts_with('d'), "dir line: {}", lines[0]);
        assert!(lines[1].ends_with("zebra.txt"));
        assert!(lines[1].starts_with('-'));
        assert!(lines[1].contains("11"), "size 11 visible: {}", lines[1]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ll_errors_on_missing_path() {
        let out = dispatch(&[
            "ll".to_string(),
            "/definitely/not/a/real/dir-tuxixi".to_string(),
        ]);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("ll:"));
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn ll_defaults_to_current_directory() {
        let out = dispatch(&["ll".to_string()]);
        // The cwd always exists in tests, so this must succeed.
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn cls_prints_the_clear_sequence() {
        let out = dispatch(&["cls".to_string()]);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, crate::lsutil::cls_string());
        assert!(out.stderr.is_empty());
    }

    #[test]
    fn ip_flag_never_panics_offline() {
        let out = dispatch(&["--ip".to_string()]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("◆ NETWORK INFORMATION"));
        // In the sandbox there is usually an internal IP but no route
        // helper binaries; either way the output must be well-formed.
        assert!(out.stdout.contains("INTERNAL IP"));
        assert!(out.stdout.contains("EXTERNAL IP"));
    }
}
