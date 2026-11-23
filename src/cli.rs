// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

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
        "theme" => run_theme_command(&args[1..], &mut out),
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
                out.stdout.push_str(&format!(
                    "{PURPLE}\u{25c6} TUXIXI UPDATE{RESET}\n",
                    PURPLE = sysinfo::PURPLE,
                    RESET = sysinfo::RESET
                ));
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

/// Handle `tuxixi theme ...`: inspect and change the active theme.
fn run_theme_command(rest: &[String], out: &mut CliOutput) {
    run_theme_command_at(rest, out, None);
}

/// The real worker; `config_override` injects the config path for tests.
fn run_theme_command_at(
    rest: &[String],
    out: &mut CliOutput,
    config_override: Option<&std::path::Path>,
) {
    let mut mgr = match config_override {
        Some(p) => crate::settings::SettingsManager::load(p),
        None => crate::settings::SettingsManager::load_default_location(),
    };
    let sub = rest.first().map(|s| s.as_str()).unwrap_or("list");

    let usage = "Usage: tuxixi theme [list | current | set <name> | next]\n".to_string();
    match sub {
        "--help" | "help" | "-h" => {
            out.stdout.push_str(&usage);
            out.stdout
                .push_str("\n  tuxixi theme             list all themes (current one marked)\n");
            out.stdout
                .push_str("  tuxixi theme current     print the active theme name\n");
            out.stdout
                .push_str("  tuxixi theme set <name>  switch the theme (persisted)\n");
            out.stdout
                .push_str("  tuxixi theme next        cycle to the next theme\n");
            out.stdout
                .push_str("\nUser themes (any format) go into ~/.config/tuxixi/themes/\n");
        }
        "list" => {
            let reg = crate::theme::Registry::global();
            let current = mgr.get().theme.clone();
            out.stdout.push_str(&format!(
                "{PURPLE}\u{25c6} TUXIXI THEMES{RESET}\n",
                PURPLE = sysinfo::PURPLE,
                RESET = sysinfo::RESET
            ));
            for name in reg.names() {
                if name == current {
                    out.stdout.push_str(&format!(
                        "  {LIME}\u{25cf} {name}{RESET}   {S}(current){RESET}\n",
                        LIME = sysinfo::LIME,
                        RESET = sysinfo::RESET,
                        S = sysinfo::SLATE
                    ));
                } else {
                    out.stdout.push_str(&format!(
                        "  {S}\u{25cb} {name}{RESET}\n",
                        S = sysinfo::SLATE,
                        RESET = sysinfo::RESET
                    ));
                }
            }
            out.stdout.push_str(&format!(
                "\n{} themes available. Change with {LIME}tuxixi theme set <name>{RESET}\nor {LIME}Ctrl+Shift+T{RESET} inside TUXIXI.\n",
                reg.len(),
                RESET = sysinfo::RESET,
                LIME = sysinfo::LIME
            ));
        }
        "current" => {
            out.stdout.push_str(&format!("{}\n", mgr.get().theme));
        }
        "set" => {
            let Some(name) = rest.get(1) else {
                out.stderr.push_str(&format!(
                    "{RED}tuxixi theme set: missing theme name{RESET}\n",
                    RED = sysinfo::RED,
                    RESET = sysinfo::RESET
                ));
                out.stderr.push_str(&usage);
                out.exit_code = 2;
                return;
            };
            let reg = crate::theme::Registry::global();
            if reg.get(name).is_none() {
                out.stderr.push_str(&format!(
                    "{RED}Unknown theme:{RESET} {name}\n",
                    RED = sysinfo::RED,
                    RESET = sysinfo::RESET
                ));
                out.stderr.push_str(&format!(
                    "Run {LIME}tuxixi theme list{RESET} to see available themes.\n",
                    LIME = sysinfo::LIME,
                    RESET = sysinfo::RESET
                ));
                out.exit_code = 2;
                return;
            }
            let mut s = mgr.get().clone();
            s.theme = name.clone();
            if mgr.set(s) {
                out.stdout.push_str(&format!(
                    "{LIME}\u{2713}{RESET} Theme set to {name}\n",
                    LIME = sysinfo::LIME,
                    RESET = sysinfo::RESET
                ));
            } else {
                out.stdout
                    .push_str(&format!("Theme '{name}' is already active.\n"));
            }
        }
        "next" => {
            if mgr.cycle_theme() {
                out.stdout.push_str(&format!(
                    "{LIME}\u{2713}{RESET} Theme: {}\n",
                    mgr.get().theme,
                    LIME = sysinfo::LIME,
                    RESET = sysinfo::RESET
                ));
            } else {
                out.stdout.push_str("Theme unchanged.\n");
            }
        }
        other => {
            out.stderr.push_str(&format!(
                "{RED}Unknown theme command:{RESET} {other}\n",
                RED = sysinfo::RED,
                RESET = sysinfo::RESET
            ));
            out.stderr.push_str(&usage);
            out.exit_code = 2;
        }
    }
}

fn gather_ip() -> sysinfo::IpInfo {
    let (internal, iface) = internal_ip_and_iface();
    let external = if internal.is_some() {
        external_ip()
    } else {
        None
    };
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
    if let Ok(output) = std::process::Command::new("hostname").arg("-I").output() {
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
        .args([
            "-fsS",
            "--connect-timeout",
            "1",
            "--max-time",
            "2",
            "https://api.ipify.org",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Exact port of `show_help`.
pub fn help_text() -> String {
    let b = sysinfo::BOLD;
    let r = sysinfo::RESET;
    let p = sysinfo::PURPLE;
    let c = sysinfo::CYAN;
    let l = sysinfo::LIME;
    let s = sysinfo::SLATE;
    let a = sysinfo::AMBER;

    let mut out = String::new();
    out.push('\n');
    out.push_str(&format!("{b}{p}◆ TUXIXI TERMINAL{r} {s}v{VERSION}{r}\n"));
    out.push_str(&format!(
        "{s}────────────────────────────────────────{r}\n\n"
    ));

    out.push_str(&format!("{b}{c}COMMANDS{r}\n"));
    out.push_str(&format!("  {l}tuxixi --help{r}      {s}Display help{r}\n"));
    out.push_str(&format!(
        "  {l}tuxixi --version{r}   {s}Version information{r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi --system{r}    {s}System information{r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi --sys{r}       {s}System information{r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi --ip{r}        {s}Network information{r}\n\n"
    ));

    out.push_str(&format!("{b}{c}UPDATES{r}\n"));
    out.push_str(&format!(
        "  {l}tuxixi update{r}           {s}Update binary + themes (hash based){r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi update --self{r}    {s}Update the tuxixi binary itself{r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi update --theme{r}   {s}Sync themes from the repository{r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi update --check{r}   {s}Only show what would change{r}\n\n"
    ));

    out.push_str(&format!("{b}{c}THEMES{r}\n"));
    out.push_str(&format!(
        "  {s}32 built-in themes (the real upstream files), default: {DEFAULT}{r}\n",
        DEFAULT = crate::theme::DEFAULT_THEME
    ));
    out.push_str(&format!(
        "  {l}tuxixi theme list{r}       {s}Show all themes (current one marked){r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi theme set <name>{r} {s}Switch the theme{r}\n"
    ));
    out.push_str(&format!(
        "  {l}tuxixi theme next{r}       {s}Cycle to the next theme{r}\n"
    ));
    out.push_str(&format!(
        "  {l}Ctrl+Shift+T{r}            {s}Cycle theme inside TUXIXI{r}\n"
    ));
    out.push_str(&format!(
        "  {s}Drop your own themes (JSON / iTerm2 .itermcolors / Alacritty{r}\n"
    ));
    out.push_str(&format!("  {s}.toml/.yml / kitty .conf) into:{r}\n"));
    out.push_str(&format!(
        "  {l}~/.config/tuxixi/themes/{r}   {s}(a cloned theme repo works too){r}\n\n"
    ));

    out.push_str(&format!("{b}{c}UTILITIES{r}\n"));
    out.push_str(&format!(
        "  {l}ll{r}                 {s}List directory contents{r}\n"
    ));
    out.push_str(&format!(
        "  {l}cls{r}                {s}Clear terminal{r}\n\n"
    ));

    out.push_str(&format!("{b}{c}KEYBOARD SHORTCUTS{r}\n"));
    out.push_str(&format!(
        "  {a}Ctrl+Shift+c{r}       {s}Copy selection{r}\n"
    ));
    out.push_str(&format!("  {a}Ctrl+Shift+V{r}       {s}Paste cleanly{r}\n"));
    out.push_str(&format!(
        "  {a}Ctrl+Shift+T{r}       {s}Change theme (live){r}\n"
    ));
    out.push_str(&format!(
        "  {a}Ctrl+Shift+G{r}       {s}Toggle glass blur{r}\n"
    ));
    out.push_str(&format!(
        "  {a}Ctrl+{r}={a}/{r}- {s}       {s}Increase / decrease font size{r}\n"
    ));
    out.push_str(&format!(
        "  {a}Shift+PageUp/Down{r}  {s}Scroll the scrollback{r}\n"
    ));
    out.push_str(&format!(
        "  {a}Mouse wheel{r}        {s}Scroll the scrollback{r}\n"
    ));
    out.push_str(&format!(
        "  {a}Mouse drag{r}         {s}Select text (release to keep){r}\n\n"
    ));

    out.push_str(&format!("{p}◆{r} {s}Built for terminal enthusiasts{r}\n\n"));
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
        assert!(
            out.stdout.contains(&format!("v{VERSION}")),
            "stdout: {}",
            out.stdout
        );
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
            "Ctrl+Shift+T",
            "Shift+PageUp/Down",
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
        assert!(out.stdout.contains("TokyoNight"));
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
    fn theme_help_prints_usage() {
        let out = dispatch(&["theme".to_string(), "--help".to_string()]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: tuxixi theme"));
        assert!(out.stdout.contains("theme set"));
        assert!(out.stdout.contains("~/.config/tuxixi/themes"));
    }

    fn temp_theme_config(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuxixi-theme-cfg-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn theme_lists_builtin_themes_with_current_marker() {
        let cfg = temp_theme_config("list");
        let mut out = CliOutput::new();
        run_theme_command_at(&[], &mut out, Some(&cfg));
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("TUXIXI THEMES"));
        assert!(out.stdout.contains("TokyoNight"));
        assert!(out.stdout.contains("Dracula"));
        assert!(
            out.stdout.contains("(current)"),
            "default must be marked: {}",
            out.stdout
        );
    }

    #[test]
    fn theme_current_prints_the_active_theme() {
        let cfg = temp_theme_config("current");
        let mut out = CliOutput::new();
        run_theme_command_at(&["current".to_string()], &mut out, Some(&cfg));
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), crate::theme::DEFAULT_THEME);
    }

    #[test]
    fn theme_set_persists_to_config() {
        let cfg = temp_theme_config("set");
        let mut out = CliOutput::new();
        run_theme_command_at(
            &["set".to_string(), "Dracula".to_string()],
            &mut out,
            Some(&cfg),
        );
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Dracula"));
        // The change must be persisted.
        let content = std::fs::read_to_string(&cfg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["theme"], "Dracula");
        // Setting it again reports "already active" without error.
        let mut out2 = CliOutput::new();
        run_theme_command_at(
            &["set".to_string(), "Dracula".to_string()],
            &mut out2,
            Some(&cfg),
        );
        assert_eq!(out2.exit_code, 0);
        assert!(out2.stdout.contains("already active"));
        std::fs::remove_dir_all(cfg.parent().unwrap()).ok();
    }

    #[test]
    fn theme_set_unknown_theme_errors() {
        let cfg = temp_theme_config("setbad");
        let mut out = CliOutput::new();
        run_theme_command_at(
            &["set".to_string(), "Nope".to_string()],
            &mut out,
            Some(&cfg),
        );
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("Unknown theme:"));
        assert!(out.stderr.contains("Nope"));
        assert!(out.stderr.contains("theme list"));
    }

    #[test]
    fn theme_set_without_name_gives_usage() {
        let cfg = temp_theme_config("setnoarg");
        let mut out = CliOutput::new();
        run_theme_command_at(&["set".to_string()], &mut out, Some(&cfg));
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("missing theme name"));
    }

    #[test]
    fn theme_next_cycles_away_from_the_default() {
        let cfg = temp_theme_config("next");
        let mut out = CliOutput::new();
        run_theme_command_at(&["next".to_string()], &mut out, Some(&cfg));
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Theme: "));
        let content = std::fs::read_to_string(&cfg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_ne!(
            parsed["theme"],
            serde_json::json!(crate::theme::DEFAULT_THEME)
        );
        // And the new theme must be a real one.
        let new_name = parsed["theme"].as_str().unwrap();
        assert!(crate::theme::Registry::global().get(new_name).is_some());
        std::fs::remove_dir_all(cfg.parent().unwrap()).ok();
    }

    #[test]
    fn theme_unknown_subcommand_errors() {
        let cfg = temp_theme_config("bogus");
        let mut out = CliOutput::new();
        run_theme_command_at(&["bogus".to_string()], &mut out, Some(&cfg));
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("Unknown theme command:"));
        assert!(out.stderr.contains("bogus"));
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
