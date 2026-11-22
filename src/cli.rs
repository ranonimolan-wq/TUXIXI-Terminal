//! Port of `bin/tuxixi`: the TUXIXI command line interface.
//!
//! All commands write into injectable writers and return an exit code,
//! which makes the whole CLI unit-testable (see the tests below).

use std::io::Write;

use crate::sysinfo;

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
        "--help" => out.stdout.push_str(&help_text()),
        "--version" => out.stdout.push_str(&sysinfo::format_version()),
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
