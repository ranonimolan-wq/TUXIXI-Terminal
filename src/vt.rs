// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! The terminal emulation engine: a fixed-size character grid, VT/ANSI
//! escape-sequence processing (via the `vte` parser crate) and a
//! hard-capped scrollback ring.
//!
//! This module replaces the GTK VTE widget. It is 100% pure Rust with
//! no system dependencies, and every behavior is unit-tested by feeding
//! raw bytes through [`Terminal::feed`].
//!
//! Memory model (the "manual" part):
//! * the scrollback is a [`BoundedDeque`] with [`Terminal::DEFAULT_SCROLLBACK_CAP`]
//!   rows; overflowing pushes evict the oldest row immediately,
//! * the OSC window title is clamped to [`Terminal::MAX_TITLE_LEN`] bytes,
//! * [`Terminal::memory_report`] exposes live allocation counts so the
//!   GUI (and the tests) can verify nothing grows unboundedly.

mod model;
mod perform;

pub use model::{
    indexed_color, Cell, Color, MemoryReport, Terminal, BOLD, DIM, INVERSE, ITALIC, UNDERLINE,
    WIDE, WIDE_SPACER,
};

#[cfg(test)]
mod tests;
