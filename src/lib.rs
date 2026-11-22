//! TUXIXI Terminal - Rust implementation.
//!
//! Everything GUI-independent lives in the root modules and is fully
//! unit-tested. The GTK4 GUI port lives under [`gui`] and is only
//! compiled when the `gui` feature is enabled (requires GTK4 and
//! VTE 3.91 development libraries).

pub mod cli;
pub mod css;
pub mod keys;
pub mod lsutil;
pub mod settings;
pub mod shell;
pub mod sysinfo;
pub mod theme;
pub mod title;
pub mod update;
pub mod wallpaper;

#[cfg(feature = "gui")]
pub mod gui;
