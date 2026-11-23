// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! The winit GUI of TUXIXI — a from-scratch rewrite of the GTK4 port.
//!
//! Why rewrite? The GTK build idled at ~240 MB: GLib main loop, the
//! GSK renderers, VTE's grid state, fontconfig/Pango caches and the
//! GObject type system all allocate on startup and never give it back.
//! Alacritty shows what a terminal can weigh, so the GUI is built the
//! same way Alacritty is:
//!
//! * windowing/event loop: `winit` (no toolkit, no main-loop runtime),
//! * rendering: **OpenGL 3.3 core** via `glutin` + `glow` — the same
//!   stack Alacritty uses; the CPU only builds geometry (`crate::gpu`)
//!   and rasterizes glyphs once into a GPU atlas, repainted only when
//!   the PTY, the input or the scroll offset actually change,
//! * glass: a translucent background alpha plus a compositor
//!   blur-behind request (`crate::glass`), so the desktop bleeds
//!   through the empty parts of the window while text stays crisp,
//! * fonts: `ab_glyph`, rasterized on demand into a bounded cache,
//! * terminal emulation: our own grid + the `vte` parser (`crate::vt`),
//! * PTY: our own fork/exec (`crate::pty`),
//! * memory: every buffer is bounded and freed explicitly
//!   (`crate::mem`, `Terminal::memory_report`, `GpuRenderer::destroy`,
//!   `Atlas::reset`).
//!
//! Shortcuts:
//! * Ctrl+Shift+C / Ctrl+Shift+V — copy selection / paste
//! * Ctrl+Shift+T — cycle theme (persisted via the settings file)
//! * Ctrl+Shift+G — toggle glass blur (persisted)
//! * Ctrl+= / Ctrl+- — font size up / down (persisted)
//! * Shift+PageUp / Shift+PageDown, mouse wheel — scrollback
//!
//! Tabs are gone for good: they were a GTK-only construct (one
//! GObject stack per tab was a large share of the idle memory). This
//! build opens one window per terminal, like Alacritty.

mod app;
mod env;
mod gl;
mod handler;
mod run;
mod shortcuts;

pub use run::run;
