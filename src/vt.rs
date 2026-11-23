// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! The terminal emulation engine, backed by the `alacritty_terminal`
//! crate (the emulation core of the Alacritty project).
//!
//! The wrapper in [`model`] translates the engine grid into the local
//! [`Cell`] / [`Color`] representation the renderers consume, clamps
//! the OSC title and surfaces the child's query responses (cursor
//! position, DA, color and clipboard requests) as [`TermEvent`]s for
//! the GUI event loop to answer over the PTY.
//!
//! Memory model:
//! * scrollback is bounded by `Config::scrolling_history`
//!   ([`Terminal::DEFAULT_SCROLLBACK_CAP`] rows; the engine trims
//!   automatically),
//! * the OSC window title is clamped to [`Terminal::MAX_TITLE_LEN`] bytes,
//! * [`Terminal::memory_report`] exposes live allocation counts so the
//!   tests can verify nothing grows unboundedly.

mod model;
#[cfg(test)]
mod tests;

pub use model::{
    indexed_color, Cell, Color, MemoryReport, TermEvent, Terminal, BOLD, DIM, INVERSE, ITALIC,
    UNDERLINE, WIDE, WIDE_SPACER,
};
