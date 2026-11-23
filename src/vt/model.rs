// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Terminal engine wrapper around `alacritty_terminal`.
//!
//! The previous hand-written `vte::Perform` implementation was replaced
//! by the battle-tested emulation core of the Alacritty project. That
//! engine handles the full DEC/xterm surface (origin mode, scroll
//! regions, wide characters, bracketed paste, kitty keyboard protocol)
//! and — critical for shell line editors such as zsh's ZLE — answers
//! cursor-position (DSR/CPR), DA and color queries through the event
//! proxy, so prompts and plugins can never desynchronize their internal
//! cursor model from the actual grid.
//!
//! The wrapper keeps the project-facing API stable: a [`Terminal`]
//! exposes the same accessors the renderers and the GUI already used
//! (`visible_row`, `cell`, `cursor`, mode queries), translating the
//! engine grid into the local [`Cell`] / [`Color`] representation that
//! the GPU renderer consumes. Responses requested by the child are
//! surfaced as [`TermEvent`]s and must be written back to the PTY by
//! the GUI event loop.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::{
    cell::Cell as AlacCell, cell::Flags as AlacFlags, ClipboardType, Config, Term,
};
use alacritty_terminal::vte::ansi::{Color as AlacColor, NamedColor, Processor, Rgb};

/// Grid/attribute color reference. `Indexed` covers the classic
/// 0-255 xterm palette (0-15 resolved from the active theme, 16-255
/// from the standard color cube); `Rgb` carries true-color SGR values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb([u8; 3]),
}

pub const BOLD: u16 = 1 << 0;
pub const DIM: u16 = 1 << 1;
pub const ITALIC: u16 = 1 << 2;
pub const UNDERLINE: u16 = 1 << 3;
pub const INVERSE: u16 = 1 << 4;
pub const WIDE: u16 = 1 << 5;
/// Second half of a wide (double-width) glyph; renderers skip it.
pub const WIDE_SPACER: u16 = 1 << 6;

/// One character cell of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: u16,
}

impl Cell {
    pub const fn blank() -> Self {
        Self {
            c: ' ',
            fg: Color::Default,
            bg: Color::Default,
            flags: 0,
        }
    }

    pub fn blank_with_bg(bg: Color) -> Self {
        Self {
            c: ' ',
            fg: Color::Default,
            bg,
            flags: 0,
        }
    }

    pub fn is_blank(&self) -> bool {
        *self == Cell::blank()
    }
}

/// Mirror of the engine's attribute flags onto the local bit flags used
/// by the renderers. Unsupported engine flags (strikethrough, hidden)
/// map to 0 so the renderer paints the glyph plainly.
fn map_flags(flags: AlacFlags) -> u16 {
    let mut out = 0u16;
    if flags.contains(AlacFlags::BOLD) {
        out |= BOLD;
    }
    if flags.contains(AlacFlags::DIM) {
        out |= DIM;
    }
    if flags.contains(AlacFlags::ITALIC) {
        out |= ITALIC;
    }
    if flags.contains(AlacFlags::UNDERLINE) {
        out |= UNDERLINE;
    }
    if flags.contains(AlacFlags::INVERSE) {
        out |= INVERSE;
    }
    if flags.contains(AlacFlags::WIDE_CHAR) {
        out |= WIDE;
    }
    if flags.contains(AlacFlags::WIDE_CHAR_SPACER) {
        out |= WIDE_SPACER;
    }
    out
}

/// Translate an engine color into the local color reference.
///
/// The default foreground/background stay [`Color::Default`] so the
/// active theme (and the glass mode) keep controlling them; every
/// indexed color below 16 is left as an index so themes resolve it per
/// frame, and 16-255 expand through the standard xterm cube.
fn map_color(color: AlacColor) -> Color {
    match color {
        AlacColor::Named(NamedColor::Foreground)
        | AlacColor::Named(NamedColor::Background)
        | AlacColor::Named(NamedColor::DimForeground)
        | AlacColor::Named(NamedColor::BrightForeground)
        | AlacColor::Named(NamedColor::Cursor) => Color::Default,
        AlacColor::Named(named) => Color::Indexed(named_index(named)),
        AlacColor::Indexed(i) => Color::Indexed(i),
        AlacColor::Spec(rgb) => Color::Rgb([rgb.r, rgb.g, rgb.b]),
    }
}

/// Classic xterm index of a named color (0-7 normal, 8-15 bright).
fn named_index(named: NamedColor) -> u8 {
    match named {
        NamedColor::Black => 0,
        NamedColor::Red => 1,
        NamedColor::Green => 2,
        NamedColor::Yellow => 3,
        NamedColor::Blue => 4,
        NamedColor::Magenta => 5,
        NamedColor::Cyan => 6,
        NamedColor::White => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        NamedColor::DimBlack => 0,
        NamedColor::DimRed => 1,
        NamedColor::DimGreen => 2,
        NamedColor::DimYellow => 3,
        NamedColor::DimBlue => 4,
        NamedColor::DimMagenta => 5,
        NamedColor::DimCyan => 6,
        NamedColor::DimWhite => 7,
        NamedColor::Foreground
        | NamedColor::Background
        | NamedColor::Cursor
        | NamedColor::DimForeground
        | NamedColor::BrightForeground => 0,
    }
}

/// Standard xterm 256-color expansion for indexes >= 16.
pub fn indexed_color(n: u8) -> Color {
    match n {
        0..=15 => Color::Indexed(n),
        16..=231 => {
            let idx = n - 16;
            let r = idx / 36;
            let g = (idx % 36) / 6;
            let b = idx % 6;
            let cube = |v: u8| if v == 0 { 0 } else { v * 40 + 55 };
            Color::Rgb([cube(r), cube(g), cube(b)])
        }
        232..=255 => {
            let gray = (n - 232) * 10 + 8;
            Color::Rgb([gray, gray, gray])
        }
    }
}

/// A response or side effect the engine requests while parsing child
/// output. The GUI event loop drains these after every [`Terminal::feed`]
/// and forwards them to the PTY / clipboard / window.
pub enum TermEvent {
    /// Bytes that must be written to the PTY (CPR/DA/color-query
    /// responses, bracketed-paste rewrites and the like).
    PtyWrite(Vec<u8>),
    /// OSC 52 "store": the child wants to set the clipboard text.
    ClipboardStore(ClipboardType, String),
    /// OSC 52 "load": the child asks for the clipboard; format the
    /// text with the closure and write the result to the PTY.
    ClipboardLoad(ClipboardType, Arc<dyn Fn(&str) -> String + Send + Sync>),
    /// Color query (OSC 4/10/11): resolve the index against the active
    /// theme, format and write to the PTY.
    ColorRequest(usize, Arc<dyn Fn(Rgb) -> String + Send + Sync>),
    /// Text-area size query (CSI 14t/18t): build the [`WindowSize`] and
    /// write the formatted answer to the PTY.
    TextAreaSizeRequest(Arc<dyn Fn(WindowSize) -> String + Send + Sync>),
}

/// Event-listener adapter: the engine pushes raw [`AlacEvent`]s into a
/// shared queue which [`Terminal::feed`] drains into [`TermEvent`]s.
#[derive(Clone, Default)]
struct EventProxy(Arc<Mutex<Vec<AlacEvent>>>);

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        if let Ok(mut queue) = self.0.lock() {
            queue.push(event);
        }
    }
}

/// Dimensions descriptor for engine (re)sizing.
struct GridSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// The complete terminal state, backed by the Alacritty engine.
pub struct Terminal {
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    inner: Term<EventProxy>,
    processor: Processor,
    raw_events: Arc<Mutex<Vec<AlacEvent>>>,
    /// Consumed engine events waiting for [`Terminal::take_events`].
    pending: Vec<TermEvent>,
}

impl Terminal {
    /// Scrollback cap passed to the engine (10 000 rows, same bound as
    /// the previous in-house ring; the engine trims automatically).
    pub const DEFAULT_SCROLLBACK_CAP: usize = 10_000;
    /// The OSC window title is clamped to this byte count.
    pub const MAX_TITLE_LEN: usize = 128;
    pub const TAB_WIDTH: u16 = 8;

    pub fn new(cols: u16, rows: u16) -> Self {
        let raw_events = Arc::new(Mutex::new(Vec::new()));
        let config = Config {
            scrolling_history: Self::DEFAULT_SCROLLBACK_CAP,
            ..Default::default()
        };
        let size = GridSize {
            columns: cols.max(1) as usize,
            screen_lines: rows.max(1) as usize,
        };
        let inner = Term::new(config, &size, EventProxy(Arc::clone(&raw_events)));
        Self {
            cols: cols.max(1),
            rows: rows.max(1),
            title: String::new(),
            inner,
            processor: Processor::new(),
            raw_events,
            pending: Vec::new(),
        }
    }

    /// Parse raw PTY output into the engine. Requests emitted by the
    /// child accumulate and are surfaced through [`Terminal::take_events`].
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.processor.advance(&mut self.inner, bytes);
        let raw = std::mem::take(
            &mut *self
                .raw_events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for event in raw {
            match event {
                AlacEvent::Title(t) => self.set_title(t),
                AlacEvent::ResetTitle => self.title.clear(),
                AlacEvent::PtyWrite(s) => self.pending.push(TermEvent::PtyWrite(s.into_bytes())),
                AlacEvent::ClipboardStore(ty, text) => {
                    self.pending.push(TermEvent::ClipboardStore(ty, text))
                }
                AlacEvent::ClipboardLoad(ty, fmt) => {
                    self.pending.push(TermEvent::ClipboardLoad(ty, fmt))
                }
                AlacEvent::ColorRequest(index, fmt) => {
                    self.pending.push(TermEvent::ColorRequest(index, fmt))
                }
                AlacEvent::TextAreaSizeRequest(fmt) => {
                    self.pending.push(TermEvent::TextAreaSizeRequest(fmt))
                }
                // Wakeup / MouseCursorDirty only trigger repaints, which
                // the GUI loop already performs after every feed. Bell is
                // not visualized yet; ChildExit/Exit are handled via PTY
                // EOF in the reader thread.
                _ => {}
            }
        }
    }

    /// Clamp and store an OSC title.
    fn set_title(&mut self, t: String) {
        let mut t = t;
        t.truncate(Self::MAX_TITLE_LEN);
        self.title = t;
    }

    /// Consume the child requests accumulated by [`Terminal::feed`].
    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut self.pending)
    }

    /// Grow/shrink the grid. Content outside the new bounds is dropped
    /// by the engine; the cursor is clamped into the new viewport.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        self.inner.resize(GridSize {
            columns: self.cols as usize,
            screen_lines: self.rows as usize,
        });
    }

    /// Copy the visible viewport (at `offset` rows back into scrollback)
    /// into `out` as local [`Cell`]s, row-major. `out` is truncated to
    /// `cols * rows`; reusing the same buffer across frames keeps the
    /// render path allocation-free.
    pub fn copy_visible(&self, offset: usize, out: &mut Vec<Cell>) {
        let (cols, rows) = (self.cols, self.rows);
        out.clear();
        let grid = self.inner.grid();
        for y in 0..rows {
            let line = Line(y as i32 - offset as i32);
            let row = &grid[line];
            for x in 0..cols {
                let src: &AlacCell = &row[Column(x as usize)];
                out.push(Cell {
                    c: src.c,
                    fg: map_color(src.fg),
                    bg: map_color(src.bg),
                    flags: map_flags(src.flags),
                });
            }
        }
    }

    /// Cursor position on the live screen: (column, row), 0-based.
    pub fn cursor(&self) -> (u16, u16) {
        let point: Point = self.inner.grid().cursor.point;
        (
            point.column.0 as u16,
            (point.line.0.max(0) as u16).min(self.rows - 1),
        )
    }

    /// Whether the cursor should be painted (DECSET 25 default state).
    pub fn cursor_visible(&self) -> bool {
        self.inner
            .mode()
            .contains(alacritty_terminal::term::TermMode::SHOW_CURSOR)
    }

    /// Scrollback rows currently available for scrolling.
    pub fn scrollback_len(&self) -> usize {
        self.inner.grid().history_size()
    }

    /// True while the alternate screen buffer is active (full-screen
    /// applications). GUI scrolling is suspended in that state.
    pub fn on_alt_screen(&self) -> bool {
        self.inner
            .mode()
            .contains(alacritty_terminal::term::TermMode::ALT_SCREEN)
    }

    /// Whether the child requested bracketed paste (DECSET 2004).
    pub fn bracketed_paste(&self) -> bool {
        self.inner
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)
    }

    /// Whether the child requested application cursor keys (DECSET 1).
    pub fn app_cursor(&self) -> bool {
        self.inner
            .mode()
            .contains(alacritty_terminal::term::TermMode::APP_CURSOR)
    }

    /// Explicit memory hygiene hook. The engine bounds scrollback by
    /// `Config::scrolling_history` and frees trimmed rows itself, so
    /// only the local state that mirrors renderable content needs
    /// attention here.
    pub fn maintain_memory(&mut self) {
        self.pending.shrink_to_fit();
    }

    /// Adjust the scrollback cap at runtime. Rows beyond the new cap
    /// are freed immediately by the engine.
    pub fn set_scrollback_cap(&mut self, cap: usize) {
        self.inner.grid_mut().update_history(cap);
    }

    /// Bounded-memory report for diagnostics and tests.
    pub fn memory_report(&self) -> MemoryReport {
        let cols = self.cols as usize;
        let rows = self.rows as usize;
        MemoryReport {
            cell_bytes: cols * rows * std::mem::size_of::<Cell>(),
            live_cells: cols * rows,
            scrollback_rows: self.inner.grid().history_size(),
            scrollback_cap: Self::DEFAULT_SCROLLBACK_CAP,
            title_bytes: self.title.capacity(),
        }
    }
}

/// Memory statistics for the diagnostics overlay and the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReport {
    pub cell_bytes: usize,
    pub live_cells: usize,
    pub scrollback_rows: usize,
    pub scrollback_cap: usize,
    pub title_bytes: usize,
}
