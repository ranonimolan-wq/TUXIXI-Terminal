//! TUXIXI Terminal - Rust implementation.
//!
//! Everything GUI-independent lives in the root modules and is fully
//! unit-tested. The winit GUI lives under [`gui`] (plus the [`pty`],
//! [`font`] and [`render`] layers it drives) and is only compiled when
//! the `gui` feature is enabled. The GUI needs NO system dev
//! libraries at build time — GTK4/VTE were removed to cut idle memory
//! from ~240 MB down to single-digit MBs; rendering is OpenGL 3.3
//! (glutin + glow, Alacritty-style) with the optional glass mode.

pub mod cli;
pub mod input;
pub mod lsutil;
pub mod mem;
pub mod settings;
pub mod shell;
pub mod sysinfo;
pub mod theme;
pub mod theme_formats;
pub mod update;
pub mod vt;

#[cfg(feature = "gui")]
pub mod font;
#[cfg(feature = "gui")]
pub mod glass;
#[cfg(feature = "gui")]
pub mod gpu;
#[cfg(feature = "gui")]
pub mod gui;
#[cfg(feature = "gui")]
pub mod pty;
#[cfg(feature = "gui")]
pub mod render;
