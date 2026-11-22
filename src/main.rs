//! `tuxixi` binary entry point.
//!
//! * `tuxixi --help | --version | --sys | --system | --ip | --welcome`
//!   -> the CLI port of `bin/tuxixi` (always available).
//! * `tuxixi update`, `tuxixi theme`, `tuxixi ll`, `tuxixi cls` ->
//!   subcommands used by
//!   the shell wrappers installed next to the binary.
//! * `tuxixi` with no arguments -> the GTK4 GUI when built with
//!   `--features gui`, otherwise a helpful error.
//! * anything else -> "Unknown command" + exit code 2 (never passed to
//!   GTK, which would try to open the argument as a file).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // CLI commands work in every build configuration.
    let cli_commands = [
        "--help",
        "help",
        "--version",
        "version",
        "--system",
        "--sys",
        "--ip",
        "--welcome",
        "update",
        "theme",
        "ll",
        "cls",
    ];
    let is_cli = args
        .first()
        .map(|a| cli_commands.contains(&a.as_str()))
        .unwrap_or(false);

    if is_cli {
        return ExitCode::from(tuxixi::cli::run_and_print(&args) as u8);
    }

    // Anything that is not a known command is an error in every build
    // configuration (never let GTK try to open unknown arguments as files).
    if !args.is_empty() {
        eprintln!(
            "{}Unknown command:{} {}",
            tuxixi::sysinfo::RED,
            tuxixi::sysinfo::RESET,
            args[0]
        );
        eprintln!(
            "Run {}\"tuxixi --help\"{} for available commands.",
            tuxixi::sysinfo::LIME,
            tuxixi::sysinfo::RESET
        );
        return ExitCode::from(2);
    }

    // No arguments: launch the GUI (or explain how to get it).
    #[cfg(feature = "gui")]
    {
        tuxixi::gui::run();
        ExitCode::SUCCESS
    }

    #[cfg(not(feature = "gui"))]
    {
        eprintln!(
            "{}TUXIXI was built without the GUI (feature `gui` not enabled).{}",
            tuxixi::sysinfo::AMBER,
            tuxixi::sysinfo::RESET
        );
        eprintln!("Build it with: cargo build --release --features gui");
        eprintln!(
            "(requires libgtk-4-dev and libvte-2.91-gtk4-dev / VTE 3.91)"
        );
        ExitCode::from(1)
    }
}
