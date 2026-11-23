//! Port of the system/network diagnostics from `bin/tuxixi`
//! (`show_sys`, `show_ip`). Gathering is separated from formatting so
//! every formatter can be tested with synthetic data.

use std::path::Path;

use crate::lsutil::human_size;

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const PURPLE: &str = "\x1b[38;2;192;132;252m";
pub const CYAN: &str = "\x1b[38;2;34;211;238m";
pub const LIME: &str = "\x1b[38;2;163;230;53m";
pub const SLATE: &str = "\x1b[38;2;148;163;184m";
pub const AMBER: &str = "\x1b[38;2;251;191;36m";
pub const RED: &str = "\x1b[38;2;248;113;113m";

// ---------------------------------------------------------------
// Gathering (thin wrappers around /proc and libc)
// ---------------------------------------------------------------

pub struct SysInfo {
    pub kernel: String,
    pub uptime: String,
    pub load_avg: String,
    pub memory: String,
    pub root_disk: String,
}

pub struct IpInfo {
    pub internal_ip: Option<String>,
    pub iface: Option<String>,
    pub external_ip: Option<String>,
}

pub fn kernel_name() -> String {
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::uname(&mut uts) };
    if rc != 0 {
        return "Unknown".to_string();
    }
    let sysname = unsafe { cstr_to_string(uts.sysname.as_ptr()) };
    let release = unsafe { cstr_to_string(uts.release.as_ptr()) };
    format!("{sysname} {release}")
}

unsafe fn cstr_to_string(ptr: *const i8) -> String {
    let cstr = std::ffi::CStr::from_ptr(ptr.cast());
    cstr.to_string_lossy().into_owned()
}

/// `uptime -p` equivalent, built from /proc/uptime seconds:
/// "up 1 week, 2 days, 3 hours, 4 minutes".
pub fn uptime_p(total_seconds: u64) -> String {
    let weeks = total_seconds / 604800;
    let days = (total_seconds % 604800) / 86400;
    let hours = (total_seconds % 86400) / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let mut parts = Vec::new();
    if weeks > 0 {
        parts.push(plural(weeks, "week"));
    }
    if days > 0 {
        parts.push(plural(days, "day"));
    }
    if hours > 0 {
        parts.push(plural(hours, "hour"));
    }
    if minutes > 0 {
        parts.push(plural(minutes, "minute"));
    }
    if parts.is_empty() {
        // uptime -p shows seconds only for uptimes below one minute.
        parts.push(plural(seconds, "second"));
    }
    if parts.is_empty() {
        return "up 0 seconds".to_string();
    }
    format!("up {}", parts.join(", "))
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("{n} {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// First three fields of /proc/loadavg, space separated.
pub fn load_avg(proc_loadavg: &str) -> Option<String> {
    let fields: Vec<&str> = proc_loadavg.split_whitespace().take(3).collect();
    if fields.len() < 3 {
        return None;
    }
    Some(fields.join(" "))
}

/// `free -h` equivalent: "<used> / <total>" human readable.
/// Linux defines used = MemTotal - MemAvailable.
pub fn memory_info(meminfo: &str) -> Option<String> {
    let mut total_kb = None;
    let mut available_kb = None;
    for line in meminfo.lines() {
        if let Some(v) = parse_meminfo_field(line, "MemTotal:") {
            total_kb = Some(v);
        } else if let Some(v) = parse_meminfo_field(line, "MemAvailable:") {
            available_kb = Some(v);
        }
    }
    let total = total_kb?;
    let used = total - available_kb.unwrap_or(0);
    Some(format!(
        "{} / {}",
        human_size(used * 1024),
        human_size(total * 1024)
    ))
}

fn parse_meminfo_field(line: &str, field: &str) -> Option<u64> {
    let rest = line.strip_prefix(field)?;
    rest.trim()
        .trim_end_matches("kB")
        .trim()
        .parse::<u64>()
        .ok()
}

/// Disk usage from a statvfs-like tuple (total_blocks, free_blocks,
/// available_blocks, block_size): "<used> / <total> (<pct>%)".
pub fn disk_info(
    total_blocks: u64,
    free_blocks: u64,
    _avail_blocks: u64,
    block_size: u64,
) -> String {
    let total = total_blocks * block_size;
    let used = (total_blocks - free_blocks) * block_size;
    let pct = if total == 0 {
        0
    } else {
        (used as f64 / total as f64 * 100.0).round() as u64
    };
    format!("{} / {} ({}%)", human_size(used), human_size(total), pct)
}

/// Real statvfs("/") wrapper for `disk_info`.
pub fn root_disk_info() -> Option<String> {
    let mut fs: libc::statvfs = unsafe { std::mem::zeroed() };
    let croot = std::ffi::CString::new("/").ok()?;
    let rc = unsafe { libc::statvfs(croot.as_ptr(), &mut fs) };
    if rc != 0 {
        return None;
    }
    Some(disk_info(fs.f_blocks, fs.f_bfree, fs.f_bavail, fs.f_frsize))
}

/// Parse `ip -4 route get 1.1.1.1` output for the `src` and `dev` fields.
pub fn parse_ip_route(route_output: &str) -> (Option<String>, Option<String>) {
    let mut src = None;
    let mut dev = None;
    let mut words = route_output.split_whitespace().peekable();
    while let Some(w) = words.next() {
        if w == "src" {
            src = words.next().map(|s| s.to_string());
        } else if w == "dev" {
            dev = words.next().map(|s| s.to_string());
        }
    }
    (src, dev)
}

// ---------------------------------------------------------------
// Formatting (exact ports of the printf layouts)
// ---------------------------------------------------------------

pub fn format_sys(info: &SysInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("\n{BOLD}{PURPLE}◆ SYSTEM STATUS{RESET}\n"));
    out.push_str(&format!(
        "{SLATE}────────────────────────────────────────{RESET}\n\n"
    ));
    out.push_str(&kv("KERNEL", &info.kernel));
    out.push_str(&kv("UPTIME", &info.uptime));
    out.push_str(&kv("LOAD AVG", &info.load_avg));
    out.push_str(&kv("MEMORY", &info.memory));
    out.push_str(&kv("ROOT DISK", &info.root_disk));
    out.push_str(&format!(
        "\n{PURPLE}◆{RESET} {SLATE}System diagnostics complete{RESET}\n\n"
    ));
    out
}

pub fn format_ip(info: &IpInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("\n{BOLD}{PURPLE}◆ NETWORK INFORMATION{RESET}\n"));
    out.push_str(&format!(
        "{SLATE}────────────────────────────────────────{RESET}\n\n"
    ));

    out.push_str(&format!("{CYAN}▸ INTERNAL IP{RESET}   "));
    match (&info.internal_ip, &info.iface) {
        (Some(ip), Some(iface)) => {
            out.push_str(&format!("{LIME}{ip}{RESET} {SLATE}({iface}){RESET}\n"))
        }
        (Some(ip), None) => out.push_str(&format!("{LIME}{ip}{RESET}\n")),
        (None, _) => out.push_str(&format!("{RED}Unavailable{RESET}\n")),
    }

    out.push_str(&format!("{CYAN}▸ EXTERNAL IP{RESET}   "));
    match (&info.external_ip, &info.internal_ip) {
        (_, None) => out.push_str(&format!("{RED}Offline{RESET}\n")),
        (Some(ext), _) => out.push_str(&format!("{LIME}{ext}{RESET}\n")),
        (None, _) => out.push_str(&format!("{RED}Unavailable{RESET}\n")),
    }

    out.push_str(&format!(
        "\n{PURPLE}◆{RESET} {SLATE}Network diagnostics complete{RESET}\n\n"
    ));
    out
}

fn kv(key: &str, value: &str) -> String {
    format!("{CYAN}▸ {key:<13}{RESET}{LIME}{value}{RESET}\n")
}

pub fn format_version() -> String {
    format!(
        "{BOLD}{PURPLE}◆ TUXIXI Terminal{RESET} v{}\n",
        crate::cli::VERSION
    )
}

pub fn format_welcome() -> String {
    format!(
        "{BOLD}{PURPLE}◆ TUXIXI Terminal{RESET} {SLATE}v{VERSION}{RESET} {RED}(rust){RESET} {SLATE}— Type{RESET} {LIME}tuxixi --help{RESET} {SLATE}to view available commands and shortcuts.{RESET}\n\n",
        VERSION = crate::cli::VERSION
    )
}

/// Gather a live SysInfo snapshot (used by the real CLI path).
pub fn gather_sys() -> SysInfo {
    SysInfo {
        kernel: kernel_name(),
        uptime: std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|c| {
                c.split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
            })
            .map(|secs| uptime_p(secs as u64))
            .unwrap_or_else(|| "Unavailable".to_string()),
        load_avg: std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|c| load_avg(&c))
            .unwrap_or_else(|| "Unavailable".to_string()),
        memory: std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|c| memory_info(&c))
            .unwrap_or_else(|| "Unavailable".to_string()),
        root_disk: root_disk_info().unwrap_or_else(|| "Unavailable".to_string()),
    }
}

/// Check a path exists helper kept for tests readability.
#[allow(dead_code)]
pub fn path_exists(p: &Path) -> bool {
    p.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_formats_weeks_days_hours_minutes() {
        assert_eq!(uptime_p(30), "up 30 seconds");
        assert_eq!(uptime_p(1800), "up 30 minutes");
        assert_eq!(uptime_p(3600), "up 1 hour");
        assert_eq!(uptime_p(3661), "up 1 hour, 1 minute");
        assert_eq!(uptime_p(90061), "up 1 day, 1 hour, 1 minute");
        assert_eq!(uptime_p(604800), "up 1 week");
        assert_eq!(uptime_p(734400), "up 1 week, 1 day, 12 hours");
    }

    #[test]
    fn load_avg_takes_first_three_fields() {
        assert_eq!(
            load_avg("0.52 0.58 0.59 1/447 12345").unwrap(),
            "0.52 0.58 0.59"
        );
        assert_eq!(load_avg("0.10 0.20"), None);
        assert_eq!(load_avg(""), None);
    }

    #[test]
    fn memory_used_is_total_minus_available() {
        let meminfo = "MemTotal:       16094776 kB\nMemFree:         6412832 kB\nMemAvailable:    9800000 kB\n";
        assert_eq!(memory_info(meminfo).unwrap(), "6.0G / 15G");
    }

    #[test]
    fn memory_without_total_is_none() {
        assert_eq!(memory_info("SwapTotal: 1 kB\n"), None);
    }

    #[test]
    fn disk_info_percent_rounds() {
        // 50 of 100 GB used -> 50%
        let out = disk_info(100 * 1024 * 1024, 50 * 1024 * 1024, 50 * 1024 * 1024, 1024);
        assert_eq!(out, "50G / 100G (50%)");
        // zero total must not divide by zero
        let out = disk_info(0, 0, 0, 4096);
        assert_eq!(out, "0 / 0 (0%)");
    }

    #[test]
    fn ip_route_extraction() {
        let sample = "1.1.1.1 via 192.168.1.1 dev eth0 src 192.168.1.42 uid 1000";
        let (src, dev) = parse_ip_route(sample);
        assert_eq!(src.as_deref(), Some("192.168.1.42"));
        assert_eq!(dev.as_deref(), Some("eth0"));
    }

    #[test]
    fn ip_route_extraction_empty() {
        let (src, dev) = parse_ip_route("");
        assert!(src.is_none());
        assert!(dev.is_none());
    }

    #[test]
    fn format_sys_contains_all_sections() {
        let info = SysInfo {
            kernel: "Linux 6.1.0".into(),
            uptime: "up 5 minutes".into(),
            load_avg: "0.1 0.2 0.3".into(),
            memory: "1.0G / 16G".into(),
            root_disk: "10G / 50G (20%)".into(),
        };
        let out = format_sys(&info);
        for needle in [
            "◆ SYSTEM STATUS",
            "KERNEL",
            "UPTIME",
            "LOAD AVG",
            "MEMORY",
            "ROOT DISK",
            "System diagnostics complete",
            "Linux 6.1.0",
        ] {
            assert!(out.contains(needle), "missing: {needle}");
        }
    }

    #[test]
    fn format_ip_variants() {
        let online = IpInfo {
            internal_ip: Some("192.168.1.42".into()),
            iface: Some("wlan0".into()),
            external_ip: Some("1.2.3.4".into()),
        };
        let out = format_ip(&online);
        assert!(out.contains("◆ NETWORK INFORMATION"));
        assert!(out.contains("192.168.1.42"));
        assert!(out.contains("(wlan0)"));
        assert!(out.contains("1.2.3.4"));

        let offline = IpInfo {
            internal_ip: None,
            iface: None,
            external_ip: None,
        };
        let out = format_ip(&offline);
        assert!(out.contains("Unavailable"));
        assert!(out.contains("Offline"));
    }

    #[test]
    fn format_version_and_welcome_show_version() {
        let v = format_version();
        assert!(v.contains("TUXIXI Terminal"));
        assert!(v.contains(&format!("v{}", crate::cli::VERSION)));
        let w = format_welcome();
        assert!(w.contains("tuxixi --help"));
        assert!(w.ends_with("\n\n"));
    }

    #[test]
    fn welcome_shows_the_rust_tag_in_red() {
        let w = format_welcome();
        assert!(
            w.contains("(rust)"),
            "welcome must advertise the Rust rewrite"
        );
        // The tag is wrapped in the red ANSI color, not plain text.
        let tagged = format!("{RED}(rust){RESET}");
        assert!(w.contains(&tagged), "the (rust) tag must be red");
        // It sits between the version and the dash.
        let marker = format!("v{}{RESET} {RED}(rust){RESET}", crate::cli::VERSION);
        assert!(w.contains(&marker), "tag must come right after the version");
    }

    #[test]
    fn kernel_name_is_nonempty() {
        assert!(!kernel_name().is_empty());
    }
}
