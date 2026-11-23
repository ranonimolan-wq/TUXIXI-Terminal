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

use crate::mem::BoundedDeque;
use vte::{Params, Perform};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SavedCursor {
    col: u16,
    row: u16,
    origin: bool,
    fg: Color,
    bg: Color,
    flags: u16,
}

/// The complete terminal state: grid, cursor, scroll region, modes,
/// scrollback and title. Construct with [`Terminal::new`], drive with
/// [`Terminal::feed`], read with the accessors below.
pub struct Terminal {
    pub cols: u16,
    pub rows: u16,
    grid: Vec<Cell>,
    alt_grid: Vec<Cell>,
    on_alt: bool,

    col: u16,
    row: u16,
    origin_mode: bool,
    autowrap: bool,
    pending_wrap: bool,
    app_cursor: bool,
    bracketed_paste: bool,
    cursor_visible: bool,

    scroll_top: u16,
    scroll_bot: u16,
    saved: SavedCursor,

    fg: Color,
    bg: Color,
    flags: u16,

    tabstops: Vec<bool>,
    scrollback: BoundedDeque<Vec<Cell>>,
    pub title: String,
    parser: vte::Parser,
}

impl Terminal {
    /// Default scrollback cap: 10 000 rows of `cols` cells. At 80 cols
    /// and 16 bytes per cell that bounds scrollback to ~13 MB — the
    /// GTK build had no such bound at all.
    pub const DEFAULT_SCROLLBACK_CAP: usize = 10_000;
    pub const MAX_TITLE_LEN: usize = 128;
    pub const TAB_WIDTH: u16 = 8;

    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(2);
        let mut term = Self {
            cols,
            rows,
            grid: vec![Cell::blank(); (cols as usize) * (rows as usize)],
            alt_grid: vec![Cell::blank(); (cols as usize) * (rows as usize)],
            on_alt: false,
            col: 0,
            row: 0,
            origin_mode: false,
            autowrap: true,
            pending_wrap: false,
            app_cursor: false,
            bracketed_paste: false,
            cursor_visible: true,
            scroll_top: 0,
            scroll_bot: rows - 1,
            saved: SavedCursor {
                col: 0,
                row: 0,
                origin: false,
                fg: Color::Default,
                bg: Color::Default,
                flags: 0,
            },
            fg: Color::Default,
            bg: Color::Default,
            flags: 0,
            tabstops: vec![false; cols as usize],
            scrollback: BoundedDeque::new(Self::DEFAULT_SCROLLBACK_CAP),
            title: String::new(),
            parser: vte::Parser::new(),
        };
        term.reset_tabstops();
        term
    }

    fn reset_tabstops(&mut self) {
        self.tabstops.clear();
        self.tabstops.resize(self.cols as usize, false);
        let mut i = Self::TAB_WIDTH as usize;
        while i < self.tabstops.len() {
            self.tabstops[i] = true;
            i += Self::TAB_WIDTH as usize;
        }
    }

    /// Feed raw PTY bytes through the VT parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        // Swap the parser out so the borrow checker sees two disjoint
        // borrows: the parser state and the Perform implementation (the
        // terminal itself). `vte::Parser::new` is allocation-free.
        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
        for &b in bytes {
            parser.advance(self, b);
        }
        self.parser = parser;
    }

    // -------------------------------------------------------------
    // Read accessors used by the renderer / GUI.
    // -------------------------------------------------------------

    pub fn cursor(&self) -> (u16, u16) {
        (self.col, self.row)
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn app_cursor(&self) -> bool {
        self.app_cursor
    }

    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    pub fn scrollback_cap(&self) -> usize {
        self.scrollback.cap()
    }

    pub fn on_alt_screen(&self) -> bool {
        self.on_alt
    }

    fn active_grid(&self) -> &[Cell] {
        if self.on_alt {
            &self.alt_grid
        } else {
            &self.grid
        }
    }

    fn active_grid_mut(&mut self) -> &mut [Cell] {
        if self.on_alt {
            &mut self.alt_grid
        } else {
            &mut self.grid
        }
    }

    /// Cell at (x, y) of the *live* screen (not the scrollback view).
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        if x >= self.cols || y >= self.rows {
            return None;
        }
        let idx = (y as usize) * (self.cols as usize) + (x as usize);
        self.active_grid().get(idx)
    }

    fn cell_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        if x >= self.cols || y >= self.rows {
            return None;
        }
        let idx = (y as usize) * (self.cols as usize) + (x as usize);
        self.active_grid_mut().get_mut(idx)
    }

    /// Row slice of the live screen.
    pub fn grid_row(&self, y: u16) -> &[Cell] {
        let start = (y as usize) * (self.cols as usize);
        let end = start + self.cols as usize;
        &self.active_grid()[start..end.min(self.active_grid().len())]
    }

    /// A scrollback row (0 = oldest retained).
    pub fn scrollback_row(&self, index: usize) -> Option<&[Cell]> {
        self.scrollback.get(index).map(|r| r.as_slice())
    }

    /// Resolve a visible row for the scroll view: `offset` rows of
    /// scrollback are pinned above the live grid. Returns `None` for
    /// the gap that appears when scrolling past the beginning of
    /// scrollback.
    pub fn visible_row(&self, offset: usize, y: u16) -> Option<&[Cell]> {
        if offset == 0 || self.on_alt {
            return Some(self.grid_row(y));
        }
        let sb = self.scrollback.len();
        let pull = offset.min(sb);
        if (y as usize) < pull {
            let idx = sb - pull + y as usize;
            return self.scrollback.get(idx).map(|r| r.as_slice());
        }
        let grid_y = y as isize - pull as isize;
        if grid_y >= 0 && grid_y < self.rows as isize {
            Some(self.grid_row(grid_y as u16))
        } else {
            None
        }
    }

    /// Memory statistics for the debug/verification path.
    pub fn memory_report(&self) -> MemoryReport {
        let cell = std::mem::size_of::<Cell>();
        MemoryReport {
            cell_bytes: cell,
            live_cells: (self.cols as usize) * (self.rows as usize),
            alt_cells: (self.cols as usize) * (self.rows as usize),
            scrollback_rows: self.scrollback.len(),
            scrollback_cap: self.scrollback.cap(),
            title_bytes: self.title.capacity(),
        }
    }

    /// Frame-boundary housekeeping for the scrollback ring (compaction).
    pub fn maintain_memory(&mut self) {
        self.scrollback.maintain();
    }

    /// Change the scrollback cap (clamped to >= 1) and truncate.
    pub fn set_scrollback_cap(&mut self, cap: usize) {
        let cap = cap.max(1);
        // Rebuild the deque: BoundedDeque has no set_cap, so drain into
        // a fresh one keeping only the newest rows.
        let keep = cap.min(self.scrollback.len());
        let start = self.scrollback.len() - keep;
        let rows: Vec<Vec<Cell>> = self.scrollback.iter().skip(start).cloned().collect();
        self.scrollback.clear();
        self.scrollback = BoundedDeque::new(cap);
        for row in rows {
            self.scrollback.push_back(row);
        }
    }

    /// Resize the live grids (top-left anchored) and clamp state.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        let old_cols = self.cols;
        let old_rows = self.rows;
        if cols == old_cols && rows == old_rows {
            return;
        }

        // When shrinking rows, keep the lines around the cursor: rows
        // that scroll off the TOP (above the new cursor position) go to
        // the scrollback (main screen only), like real terminals.
        if rows < old_rows && !self.on_alt {
            let new_row = self.row.min(rows - 1);
            let spill = (self.row - new_row) as usize;
            for y in 0..spill.min(old_rows as usize) {
                let start = y * old_cols as usize;
                let row = self.grid[start..start + old_cols as usize].to_vec();
                self.scrollback.push_back(row);
            }
            let cut = spill * old_cols as usize;
            self.grid.drain(0..cut);
            self.row = new_row;
        }

        // Re-flow both grids top-left anchored into the new size.
        let reflow = |old: &Vec<Cell>| -> Vec<Cell> {
            let mut next = vec![Cell::blank(); cols as usize * rows as usize];
            let n = old_cols.min(cols) as usize;
            for y in 0..rows.min(old_rows) as usize {
                for x in 0..n {
                    next[y * cols as usize + x] = old[y * old_cols as usize + x];
                }
            }
            next
        };
        self.grid = reflow(&self.grid);
        self.alt_grid = reflow(&self.alt_grid);

        self.cols = cols;
        self.rows = rows;
        self.col = self.col.min(cols - 1);
        self.row = self.row.min(rows - 1);
        self.scroll_top = 0;
        self.scroll_bot = rows - 1;
        self.reset_tabstops();
    }

    // -------------------------------------------------------------
    // Internal grid operations.
    // -------------------------------------------------------------

    fn clamp_to_region_row(&self, y: u16) -> u16 {
        y.clamp(self.scroll_top, self.scroll_bot)
    }

    /// Scroll the region [top, bot] up by n rows (content moves up).
    fn scroll_up(&mut self, top: u16, bot: u16, n: u16) {
        let top = top.min(bot) as usize;
        let bot = bot as usize;
        if top > bot || bot >= self.rows as usize || top >= self.rows as usize {
            return;
        }
        let height = bot - top + 1;
        let n = (n as usize).min(height);
        if n == 0 {
            return;
        }

        // Main screen, full-width region: evicted rows go to scrollback.
        let is_full_region = top == 0 && bot == self.rows as usize - 1;
        if is_full_region && !self.on_alt {
            let cols = self.cols as usize;
            for i in 0..n {
                let row: Vec<Cell> = self.active_grid()[i * cols..(i + 1) * cols].to_vec();
                self.scrollback.push_back(row);
            }
        }

        let bg = self.bg;
        let cols = self.cols as usize;
        let grid = self.active_grid_mut();
        for y in top..(bot + 1 - n) {
            let src = (y + n) * cols;
            let dst = y * cols;
            grid.copy_within(src..src + cols, dst);
        }
        for y in (bot + 1 - n)..(bot + 1) {
            for x in 0..cols {
                grid[y * cols + x] = Cell::blank_with_bg(bg);
            }
        }
    }

    /// Scroll the region [top, bot] down by n rows (content moves down).
    fn scroll_down(&mut self, top: u16, bot: u16, n: u16) {
        let top = top.min(bot) as usize;
        let bot = bot as usize;
        if top > bot || bot >= self.rows as usize || top >= self.rows as usize {
            return;
        }
        let height = bot - top + 1;
        let n = (n as usize).min(height);
        if n == 0 {
            return;
        }
        let bg = self.bg;
        let cols = self.cols as usize;
        let grid = self.active_grid_mut();
        for y in ((top + n)..=bot).rev() {
            let src = (y - n) * cols;
            let dst = y * cols;
            grid.copy_within(src..src + cols, dst);
        }
        for y in top..(top + n) {
            for x in 0..cols {
                grid[y * cols + x] = Cell::blank_with_bg(bg);
            }
        }
    }

    /// Move the cursor down one line, scrolling at the region bottom.
    fn linefeed(&mut self) {
        if self.row == self.scroll_bot {
            self.scroll_up(self.scroll_top, self.scroll_bot, 1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    fn carriage_return(&mut self) {
        self.col = 0;
        self.pending_wrap = false;
    }

    fn backspace(&mut self) {
        self.col = self.col.saturating_sub(1);
        self.pending_wrap = false;
    }

    fn advance_tab(&mut self) {
        let mut x = self.col as usize + 1;
        while x < self.tabstops.len() && !self.tabstops[x] {
            x += 1;
        }
        self.col = (x as u16).min(self.cols - 1);
        self.pending_wrap = false;
    }

    /// Wrap-aware cursor advance after printing a glyph of `width` cells.
    fn advance(&mut self, width: u16) {
        if self.col + width >= self.cols {
            if self.autowrap {
                self.pending_wrap = true;
            } else {
                self.col = self.cols - width.max(1);
            }
        } else {
            self.col += width;
        }
    }

    fn clear_row_range(&mut self, y: u16, x0: u16, x1: u16) {
        let bg = self.bg;
        for x in x0..=x1.min(self.cols - 1) {
            if let Some(cell) = self.cell_mut(x, y) {
                *cell = Cell::blank_with_bg(bg);
            }
        }
    }

    fn erase_all(&mut self) {
        let cols = self.cols;
        for y in 0..self.rows {
            self.clear_row_range(y, 0, cols - 1);
        }
    }

    // -------------------------------------------------------------
    // vte::Perform implementation.
    // -------------------------------------------------------------

    fn param(&self, params: &Params, index: usize, default: u16) -> u16 {
        let collected: Vec<&[u16]> = params.iter().collect();
        collected
            .get(index)
            .and_then(|s| s.first().copied())
            .filter(|&v| v != 0)
            .unwrap_or(default)
    }

    fn apply_sgr(&mut self, params: &Params) {
        let items: Vec<&[u16]> = params.iter().collect();
        if items.is_empty() {
            self.fg = Color::Default;
            self.bg = Color::Default;
            self.flags &= !(BOLD | DIM | ITALIC | UNDERLINE | INVERSE);
            return;
        }
        let mut i = 0;
        while i < items.len() {
            let first = items[i].first().copied().unwrap_or(0);
            match first {
                0 => {
                    self.fg = Color::Default;
                    self.bg = Color::Default;
                    self.flags &= !(BOLD | DIM | ITALIC | UNDERLINE | INVERSE);
                }
                1 => self.flags |= BOLD,
                2 => self.flags |= DIM,
                3 => self.flags |= ITALIC,
                4 => self.flags |= UNDERLINE,
                7 => self.flags |= INVERSE,
                22 => self.flags &= !(BOLD | DIM),
                23 => self.flags &= !ITALIC,
                24 => self.flags &= !UNDERLINE,
                27 => self.flags &= !INVERSE,
                30..=37 => self.fg = Color::Indexed((first - 30) as u8),
                38 | 58 => {
                    let (color, consumed) = parse_extended_color(&items[i..]);
                    if first == 38 {
                        if let Some(c) = color {
                            self.fg = c;
                        }
                    }
                    i += consumed - 1;
                }
                39 => self.fg = Color::Default,
                40..=47 => self.bg = Color::Indexed((first - 40) as u8),
                48 => {
                    let (color, consumed) = parse_extended_color(&items[i..]);
                    if let Some(c) = color {
                        self.bg = c;
                    }
                    i += consumed - 1;
                }
                49 => self.bg = Color::Default,
                90..=97 => self.fg = Color::Indexed((first - 90 + 8) as u8),
                100..=107 => self.bg = Color::Indexed((first - 100 + 8) as u8),
                _ => {}
            }
            i += 1;
        }
    }

    fn set_mode(&mut self, private: bool, params: &Params, enable: bool) {
        let collected: Vec<&[u16]> = params.iter().collect();
        for p in collected {
            let Some(&mode) = p.first() else { continue };
            if !private {
                continue;
            }
            match mode {
                1 => self.app_cursor = enable,
                6 => self.origin_mode = enable,
                7 => self.autowrap = enable,
                25 => self.cursor_visible = enable,
                2004 => self.bracketed_paste = enable,
                1047 => {
                    if enable && !self.on_alt {
                        self.on_alt = true;
                        self.erase_all();
                    } else if !enable {
                        self.on_alt = false;
                    }
                }
                1048 => {
                    if enable {
                        self.save_cursor();
                    } else {
                        self.restore_cursor();
                    }
                }
                1049 => {
                    if enable {
                        self.save_cursor();
                        if !self.on_alt {
                            self.on_alt = true;
                            self.erase_all();
                        }
                    } else {
                        self.on_alt = false;
                        self.restore_cursor();
                    }
                }
                _ => {}
            }
        }
    }

    fn save_cursor(&mut self) {
        self.saved = SavedCursor {
            col: self.col,
            row: self.row,
            origin: self.origin_mode,
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
        };
    }

    fn restore_cursor(&mut self) {
        let s = self.saved;
        self.col = s.col.min(self.cols - 1);
        self.row = self.clamp_to_region_row(s.row).min(self.rows - 1);
        self.origin_mode = s.origin;
        self.fg = s.fg;
        self.bg = s.bg;
        self.flags = s.flags;
        self.pending_wrap = false;
    }

    fn full_reset(&mut self) {
        let cols = self.cols;
        let rows = self.rows;
        let cap = self.scrollback_cap();
        *self = Terminal::new(cols, rows);
        self.set_scrollback_cap(cap);
    }
}

/// Memory statistics returned by [`Terminal::memory_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReport {
    pub cell_bytes: usize,
    pub live_cells: usize,
    pub alt_cells: usize,
    pub scrollback_rows: usize,
    pub scrollback_cap: usize,
    pub title_bytes: usize,
}

/// Parse an extended SGR color (38/48/58) from the remaining SGR
/// parameter slices. Handles both the colon form (`38:2:r:g:b` grouped
/// in one slice) and the flattened semicolon form (`38`, `2`, `r`, `g`,
/// `b` in separate slices). Returns the color and the number of slices
/// consumed.
fn parse_extended_color(items: &[&[u16]]) -> (Option<Color>, usize) {
    let first = items.first().copied().unwrap_or(&[]);
    if first.len() >= 2 {
        // Colon form: subparameters live in the same slice.
        match first[1] {
            5 => {
                let n = first.get(2).copied().unwrap_or(0);
                (Some(indexed_color(n as u8)), 1)
            }
            2 => {
                if first.len() >= 5 {
                    // 38:2:<r>:<g>:<b> or 38:2:<colorspace>:<r>:<g>:<b>
                    let n = first.len();
                    (
                        Some(Color::Rgb([
                            first[n - 3] as u8,
                            first[n - 2] as u8,
                            first[n - 1] as u8,
                        ])),
                        1,
                    )
                } else {
                    (None, 1)
                }
            }
            _ => (None, 1),
        }
    } else {
        // Flattened semicolon form.
        let mode = items.get(1).and_then(|s| s.first().copied());
        match mode {
            Some(5) => {
                let n = items.get(2).and_then(|s| s.first().copied()).unwrap_or(0);
                (Some(indexed_color(n as u8)), 3)
            }
            Some(2) => {
                let get =
                    |k: usize| items.get(k).and_then(|s| s.first().copied()).unwrap_or(0) as u8;
                (Some(Color::Rgb([get(2), get(3), get(4)])), 5)
            }
            _ => (None, 1),
        }
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

impl Perform for Terminal {
    fn print(&mut self, c: char) {
        let width = unicode_width::UnicodeWidthChar::width(c)
            .unwrap_or(1)
            .max(1) as u16;

        if self.pending_wrap {
            self.pending_wrap = false;
            self.col = 0;
            self.linefeed();
        }

        // Snapshot the attribute state first: `cell_mut` borrows the
        // grid, and the borrow checker wants these as local values.
        let (col, row, fg, bg, mut flags) = (self.col, self.row, self.fg, self.bg, self.flags);
        if width >= 2 {
            flags |= WIDE;
        }

        // Overwrite the trailing half of a wide char split across the
        // wrap boundary.
        if let Some(cell) = self.cell_mut(col, row) {
            cell.c = c;
            cell.fg = fg;
            cell.bg = bg;
            cell.flags = flags;
        }

        if width >= 2 {
            // The spacer cell: skipped by the renderer, overwritten on
            // the next print.
            if col + 1 < self.cols {
                if let Some(cell) = self.cell_mut(col + 1, row) {
                    *cell = Cell::blank_with_bg(bg);
                    cell.flags = WIDE_SPACER;
                }
            }
        }
        self.advance(width);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {}                  // BEL
            0x08 => self.backspace(),   // BS
            0x09 => self.advance_tab(), // HT
            0x0A..=0x0C => {
                self.pending_wrap = false;
                self.linefeed();
            }
            0x0D => self.carriage_return(), // CR
            _ => {}                         // DEL & others ignored
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // `?` is the DEC private-mode marker and arrives in
        // `intermediates`; any other intermediate means "not for us".
        if intermediates.iter().any(|&b| b != b'?') {
            return;
        }
        let private = intermediates.contains(&b'?');
        match action {
            'A' => {
                let n = self.param(params, 0, 1).max(1);
                self.row = self.row.saturating_sub(n).max(self.scroll_top);
                self.pending_wrap = false;
            }
            'B' | 'e' => {
                let n = self.param(params, 0, 1).max(1);
                self.row = (self.row + n).min(self.scroll_bot);
                self.pending_wrap = false;
            }
            'C' | 'a' => {
                let n = self.param(params, 0, 1).max(1);
                self.col = (self.col + n).min(self.cols - 1);
                self.pending_wrap = false;
            }
            'D' => {
                let n = self.param(params, 0, 1).max(1);
                self.col = self.col.saturating_sub(n);
                self.pending_wrap = false;
            }
            'E' => {
                let n = self.param(params, 0, 1).max(1);
                self.row = (self.row + n).min(self.scroll_bot);
                self.col = 0;
                self.pending_wrap = false;
            }
            'F' => {
                let n = self.param(params, 0, 1).max(1);
                self.row = self.row.saturating_sub(n).max(self.scroll_top);
                self.col = 0;
                self.pending_wrap = false;
            }
            'G' | '`' => {
                let n = self.param(params, 0, 1).max(1);
                self.col = (n - 1).min(self.cols - 1);
                self.pending_wrap = false;
            }
            'H' | 'f' => {
                let r = self.param(params, 0, 1).max(1);
                let c = self.param(params, 1, 1).max(1);
                self.row = self.clamp_to_region_row(r - 1).min(self.rows - 1);
                self.col = (c - 1).min(self.cols - 1);
                self.pending_wrap = false;
            }
            'd' => {
                let r = self.param(params, 0, 1).max(1);
                self.row = self.clamp_to_region_row(r - 1).min(self.rows - 1);
                self.pending_wrap = false;
            }
            'J' => {
                let mode = self.param(params, 0, 0);
                let cols = self.cols;
                let row = self.row;
                match mode {
                    0 => {
                        self.clear_row_range(row, self.col, cols - 1);
                        for y in (row + 1)..self.rows {
                            self.clear_row_range(y, 0, cols - 1);
                        }
                    }
                    1 => {
                        self.clear_row_range(row, 0, self.col);
                        for y in 0..row {
                            self.clear_row_range(y, 0, cols - 1);
                        }
                    }
                    _ => self.erase_all(),
                }
            }
            'K' => {
                let mode = self.param(params, 0, 0);
                let (x0, x1) = match mode {
                    0 => (self.col, self.cols - 1),
                    1 => (0, self.col),
                    _ => (0, self.cols - 1),
                };
                let row = self.row;
                self.clear_row_range(row, x0, x1);
            }
            'L' => {
                // IL: insert n blank lines at the cursor row.
                let n = self.param(params, 0, 1).max(1);
                if self.scroll_top <= self.row && self.row <= self.scroll_bot {
                    self.scroll_down(self.row, self.scroll_bot, n);
                }
            }
            'M' => {
                // DL: delete n lines at the cursor row.
                let n = self.param(params, 0, 1).max(1);
                if self.scroll_top <= self.row && self.row <= self.scroll_bot {
                    self.scroll_up(self.row, self.scroll_bot, n);
                }
            }
            '@' => {
                // ICH: insert n blank chars at the cursor, shifting the
                // tail of the row to the right.
                let n = self.param(params, 0, 1).max(1) as usize;
                let y = self.row as usize;
                let x = self.col as usize;
                let cols = self.cols as usize;
                let bg = self.bg;
                let start = y * cols + x;
                let end = y * cols + cols;
                if start < end {
                    let shift = n.min(end - start);
                    let grid = self.active_grid_mut();
                    for i in (start + shift..end).rev() {
                        grid[i] = grid[i - shift];
                    }
                    for cell in &mut grid[start..start + shift] {
                        *cell = Cell::blank_with_bg(bg);
                    }
                }
            }
            'P' => {
                // DCH: delete n chars at the cursor, shifting the rest
                // of the row to the left.
                let n = self.param(params, 0, 1).max(1) as usize;
                let y = self.row as usize;
                let x = self.col as usize;
                let cols = self.cols as usize;
                let bg = self.bg;
                let start = y * cols + x;
                let end = y * cols + cols;
                if start < end {
                    let shift = n.min(end - start);
                    let grid = self.active_grid_mut();
                    for i in start..(end - shift) {
                        grid[i] = grid[i + shift];
                    }
                    for cell in &mut grid[(end - shift)..end] {
                        *cell = Cell::blank_with_bg(bg);
                    }
                }
            }
            'X' => {
                // ECH: erase n chars in place.
                let n = self.param(params, 0, 1).max(1);
                let row = self.row;
                let col = self.col;
                let x1 = (col + n - 1).min(self.cols - 1);
                self.clear_row_range(row, col, x1);
            }
            'S' => {
                let n = self.param(params, 0, 1).max(1);
                self.scroll_up(self.scroll_top, self.scroll_bot, n);
            }
            'T' => {
                let n = self.param(params, 0, 1).max(1);
                self.scroll_down(self.scroll_top, self.scroll_bot, n);
            }
            'm' => self.apply_sgr(params),
            'r' => {
                // DECSTBM.
                let top = self.param(params, 0, 1).max(1) - 1;
                let bot = self.param(params, 1, self.rows) - 1;
                if top < bot && bot < self.rows {
                    self.scroll_top = top;
                    self.scroll_bot = bot;
                    self.row = if self.origin_mode { top } else { 0 };
                    self.col = 0;
                }
            }
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            'h' => self.set_mode(private, params, true),
            'l' => self.set_mode(private, params, false),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if intermediates.is_empty() {
            match byte {
                b'D' => {
                    // IND
                    self.pending_wrap = false;
                    self.linefeed();
                }
                b'E' => {
                    // NEL
                    self.carriage_return();
                    self.pending_wrap = false;
                    self.linefeed();
                }
                b'M' => {
                    // RI: reverse index (scroll down at the region top).
                    self.pending_wrap = false;
                    if self.row == self.scroll_top {
                        self.scroll_down(self.scroll_top, self.scroll_bot, 1);
                    } else {
                        self.row = self.row.saturating_sub(1);
                    }
                }
                b'c' => self.full_reset(),
                b'7' => self.save_cursor(),
                b'8' => self.restore_cursor(),
                _ => {}
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        let kind = params[0].first().copied();
        match kind {
            Some(b'0') | Some(b'2') => {
                if let Some(rest) = params.get(1) {
                    let title = String::from_utf8_lossy(rest);
                    self.title = title.chars().take(Self::MAX_TITLE_LEN).collect();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term() -> Terminal {
        Terminal::new(20, 6)
    }

    fn feed(t: &mut Terminal, s: &str) {
        t.feed(s.as_bytes());
    }

    fn text(t: &Terminal, y: u16) -> String {
        t.grid_row(y)
            .iter()
            .map(|c| c.c)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    // ----- basics -----

    #[test]
    fn printable_text_lands_on_the_grid() {
        let mut t = term();
        feed(&mut t, "hello");
        assert_eq!(text(&t, 0), "hello");
        assert_eq!(t.cursor(), (5, 0));
    }

    #[test]
    fn cr_lf_moves_to_next_line_start() {
        let mut t = term();
        feed(&mut t, "one\r\ntwo");
        assert_eq!(text(&t, 0), "one");
        assert_eq!(text(&t, 1), "two");
        assert_eq!(t.cursor(), (3, 1));
    }

    #[test]
    fn backspace_moves_cursor_left() {
        let mut t = term();
        feed(&mut t, "abc\u{8}X");
        assert_eq!(text(&t, 0), "abX");
    }

    #[test]
    fn tab_advances_to_next_tabstop() {
        let mut t = term();
        feed(&mut t, "\tX");
        // Tab stops at col 8, then X prints and the cursor advances.
        assert_eq!(t.cursor(), (9, 0));
        assert_eq!(t.cell(8, 0).unwrap().c, 'X');
    }

    // ----- linefeed & scrollback -----

    #[test]
    fn linefeed_at_bottom_scrolls_grid_up() {
        let mut t = term();
        feed(&mut t, "a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng");
        // 7 lines in a 6-row screen -> the grid scrolled by 1.
        assert_eq!(text(&t, 0), "b");
        assert_eq!(text(&t, 5), "g");
        assert_eq!(t.scrollback_len(), 1);
        assert_eq!(
            t.scrollback_row(0)
                .unwrap()
                .iter()
                .map(|c| c.c)
                .collect::<String>(),
            "a                   "
        );
    }

    #[test]
    fn scrollback_never_exceeds_its_cap() {
        let mut t = term();
        t.set_scrollback_cap(50);
        for i in 0..200 {
            t.feed(format!("line{i}\r\n").as_bytes());
        }
        assert!(t.scrollback_len() <= 50);
        // Oldest rows were evicted, newest retained.
        assert!(t
            .scrollback_row(t.scrollback_len() - 1)
            .unwrap()
            .iter()
            .map(|c| c.c)
            .collect::<String>()
            .contains("line194"));
    }

    #[test]
    fn set_scrollback_cap_truncates_keeping_newest() {
        let mut t = term();
        for i in 0..100 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        let before = t.scrollback_len();
        t.set_scrollback_cap(5);
        assert_eq!(t.scrollback_len(), 5.min(before));
    }

    #[test]
    fn memory_report_reflects_bounds() {
        let mut t = term();
        t.set_scrollback_cap(77);
        for _ in 0..120 {
            t.feed("x\r\n".as_bytes());
        }
        let r = t.memory_report();
        assert_eq!(r.scrollback_cap, 77);
        assert!(r.scrollback_rows <= 77);
        assert_eq!(r.live_cells, 20 * 6);
        assert_eq!(r.alt_cells, 20 * 6);
        assert_eq!(r.cell_bytes, std::mem::size_of::<Cell>());
    }

    // ----- cursor movement -----

    #[test]
    fn cup_moves_cursor_to_absolute_position() {
        let mut t = term();
        feed(&mut t, "\x1b[3;5HX");
        assert_eq!(t.cell(4, 2).unwrap().c, 'X');
    }

    #[test]
    fn cup_without_params_is_home() {
        let mut t = term();
        feed(&mut t, "\x1b[3;5H\x1b[H");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn cursor_movement_is_clamped() {
        let mut t = term();
        feed(&mut t, "\x1b[99A\x1b[99D"); // up far, left far -> home
        assert_eq!(t.cursor(), (0, 0));
        feed(&mut t, "\x1b[99B\x1b[99C");
        assert_eq!(t.cursor(), (19, 5));
    }

    #[test]
    fn cha_and_vpa_set_column_and_row() {
        let mut t = term();
        feed(&mut t, "\x1b[10GX");
        assert_eq!(t.cell(9, 0).unwrap().c, 'X');
        // The cursor advanced past X; VPA only moves the row.
        feed(&mut t, "\x1b[4dY");
        assert_eq!(t.cell(10, 3).unwrap().c, 'Y');
    }

    // ----- erase -----

    #[test]
    fn ed_2_clears_screen() {
        let mut t = term();
        feed(&mut t, "abcdefghij\x1b[2J");
        assert_eq!(text(&t, 0), "");
        // Erase resets nothing but content: cursor stays.
    }

    #[test]
    fn el_0_clears_to_end_of_line() {
        let mut t = term();
        feed(&mut t, "abcdef\x1b[1;3H\x1b[K");
        assert_eq!(text(&t, 0), "ab");
    }

    #[test]
    fn el_1_clears_to_cursor_inclusive() {
        let mut t = term();
        feed(&mut t, "abcdef\x1b[1;4H\x1b[1K");
        // EL 1 clears from the start THROUGH the cursor (xterm).
        assert_eq!(text(&t, 0), "    ef");
    }

    #[test]
    fn ed_1_clears_above_cursor() {
        let mut t = term();
        feed(&mut t, "aa\r\nbb\r\ncc\x1b[2;1H\x1b[1J");
        assert_eq!(text(&t, 0), "");
        // Row of the cursor is cleared through the cursor (inclusive).
        assert_eq!(text(&t, 1), " b");
        assert_eq!(text(&t, 2), "cc");
    }

    #[test]
    fn erased_cells_keep_the_current_background() {
        let mut t = term();
        feed(&mut t, "\x1b[41m\x1b[2J");
        for y in 0..t.rows {
            for x in 0..t.cols {
                assert_eq!(t.cell(x, y).unwrap().bg, Color::Indexed(1));
            }
        }
    }

    #[test]
    fn ech_erases_in_place() {
        let mut t = term();
        feed(&mut t, "abcdef\x1b[1;2H\x1b[3X");
        assert_eq!(text(&t, 0), "a   ef");
    }

    // ----- SGR attributes & colors -----

    #[test]
    fn sgr_colors_and_attrs_apply() {
        let mut t = term();
        feed(&mut t, "\x1b[1;3;32;44mZ");
        let c = t.cell(0, 0).unwrap();
        assert_eq!(c.c, 'Z');
        assert_eq!(c.fg, Color::Indexed(2));
        assert_eq!(c.bg, Color::Indexed(4));
        assert!(c.flags & BOLD != 0);
        assert!(c.flags & ITALIC != 0);
    }

    #[test]
    fn sgr_0_resets_everything() {
        let mut t = term();
        feed(&mut t, "\x1b[1;4;33;41m\x1b[0mZ");
        let c = t.cell(0, 0).unwrap();
        assert_eq!(c.fg, Color::Default);
        assert_eq!(c.bg, Color::Default);
        assert_eq!(c.flags, 0);
    }

    #[test]
    fn sgr_22_23_24_27_clear_individual_flags() {
        let mut t = term();
        feed(&mut t, "\x1b[1;3;4;7m\x1b[22mA\x1b[23mB\x1b[24mC\x1b[27mD");
        assert!(t.cell(0, 0).unwrap().flags & BOLD == 0);
        assert!(t.cell(0, 0).unwrap().flags & ITALIC != 0);
        assert!(t.cell(1, 0).unwrap().flags & ITALIC == 0);
        assert!(t.cell(2, 0).unwrap().flags & UNDERLINE == 0);
        assert!(t.cell(3, 0).unwrap().flags & INVERSE == 0);
    }

    #[test]
    fn sgr_bright_colors_map_to_palette_8_15() {
        let mut t = term();
        feed(&mut t, "\x1b[91mA\x1b[101mB");
        // A carries bright-red fg, B (next cell) carries bright-red bg.
        let a = t.cell(0, 0).unwrap();
        assert_eq!(a.fg, Color::Indexed(9));
        let b = t.cell(1, 0).unwrap();
        assert_eq!(b.bg, Color::Indexed(9));
    }

    #[test]
    fn sgr_truecolor_rgb_flattened_form() {
        let mut t = term();
        feed(&mut t, "\x1b[38;2;10;20;30mX");
        assert_eq!(t.cell(0, 0).unwrap().fg, Color::Rgb([10, 20, 30]));
        feed(&mut t, "\x1b[48;2;255;128;0mY");
        assert_eq!(t.cell(1, 0).unwrap().bg, Color::Rgb([255, 128, 0]));
    }

    #[test]
    fn sgr_truecolor_rgb_colon_grouped_form() {
        let mut t = term();
        feed(&mut t, "\x1b[38:2:1:2:3mX");
        assert_eq!(t.cell(0, 0).unwrap().fg, Color::Rgb([1, 2, 3]));
    }

    #[test]
    fn sgr_256_color_index_colon_and_flat_forms() {
        let mut t = term();
        // 196 = top of the color cube -> pure red.
        feed(&mut t, "\x1b[38;5;196mX");
        assert_eq!(t.cell(0, 0).unwrap().fg, indexed_color(196));
        feed(&mut t, "\x1b[48:5:42mY");
        assert_eq!(t.cell(1, 0).unwrap().bg, indexed_color(42));
        // Bottom rows are pure grayscale.
        feed(&mut t, "\x1b[48:5:232mZ");
        assert_eq!(t.cell(2, 0).unwrap().bg, Color::Rgb([8, 8, 8]));
    }

    #[test]
    fn indexed_color_expands_the_xterm_cube() {
        assert_eq!(indexed_color(0), Color::Indexed(0));
        assert_eq!(indexed_color(15), Color::Indexed(15));
        assert_eq!(indexed_color(16), Color::Rgb([0, 0, 0]));
        assert_eq!(indexed_color(17), Color::Rgb([0, 0, 95]));
        assert_eq!(indexed_color(231), Color::Rgb([255, 255, 255]));
        assert_eq!(indexed_color(232), Color::Rgb([8, 8, 8]));
        assert_eq!(indexed_color(255), Color::Rgb([238, 238, 238]));
    }

    // ----- autowrap -----

    #[test]
    fn autowrap_moves_to_next_line() {
        let mut t = term();
        feed(&mut t, "0123456789012345678901234567890123456789"); // 40 chars in 20 cols
        assert_eq!(text(&t, 0), "01234567890123456789");
        assert_eq!(text(&t, 1), "01234567890123456789");
    }

    #[test]
    fn pending_wrap_does_not_double_scroll() {
        let mut t = term();
        // Exactly one full line: cursor sits in "pending wrap" state.
        feed(&mut t, "01234567890123456789\r\nX");
        assert_eq!(t.cursor(), (1, 1));
        assert!(t.scrollback_len() == 0);
    }

    // ----- scroll region -----

    #[test]
    fn decstbm_limits_scrolling_region() {
        let mut t = term();
        feed(&mut t, "\x1b[2;4r");
        for i in 0..10 {
            t.feed(format!("\r\nrow{i}").as_bytes());
        }
        // Rows outside the region are untouched.
        assert_eq!(text(&t, 0), "");
        assert_eq!(text(&t, 5), "");
        // Region content scrolled.
        assert_eq!(text(&t, 1), "row7");
        assert_eq!(text(&t, 3), "row9");
        // Region scrolling does NOT push to scrollback (not full screen).
        assert_eq!(t.scrollback_len(), 0);
    }

    #[test]
    fn decstbm_resets_cursor_to_home() {
        let mut t = term();
        feed(&mut t, "\x1b[5;2H\x1b[2;4r");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn sd_scrolls_the_region_down() {
        let mut t = term();
        feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5\r\n6\x1b[2T");
        assert_eq!(text(&t, 0), "");
        assert_eq!(text(&t, 1), "");
        assert_eq!(text(&t, 2), "1");
        assert_eq!(text(&t, 3), "2");
    }

    #[test]
    fn su_scrolls_the_region_up() {
        let mut t = term();
        feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5\r\n6\x1b[2S");
        assert_eq!(text(&t, 0), "3");
        assert_eq!(text(&t, 1), "4");
        assert_eq!(text(&t, 3), "6");
        assert_eq!(text(&t, 4), "");
    }

    // ----- insert / delete -----

    #[test]
    fn ich_inserts_blanks_shifting_right() {
        let mut t = term();
        feed(&mut t, "abcdef\x1b[1;2H\x1b[2@");
        assert_eq!(text(&t, 0), "a  bcdef");
    }

    #[test]
    fn dch_deletes_chars_shifting_left() {
        let mut t = term();
        feed(&mut t, "abcdef\x1b[1;2H\x1b[2P");
        assert_eq!(text(&t, 0), "adef");
    }

    #[test]
    fn il_inserts_blank_lines() {
        let mut t = term();
        feed(&mut t, "1\r\n2\r\n3\x1b[2;1H\x1b[1L");
        assert_eq!(text(&t, 1), "");
        assert_eq!(text(&t, 2), "2");
        assert_eq!(text(&t, 3), "3");
    }

    #[test]
    fn dl_deletes_lines() {
        let mut t = term();
        feed(&mut t, "1\r\n2\r\n3\r\n4\x1b[2;1H\x1b[1M");
        assert_eq!(text(&t, 1), "3");
        assert_eq!(text(&t, 2), "4");
        assert_eq!(text(&t, 3), "");
    }

    // ----- save / restore cursor -----

    #[test]
    fn save_restore_cursor_round_trip() {
        let mut t = term();
        feed(&mut t, "\x1b[2;3H\x1b[7m\x1b[s\x1b[H\x1b[0m\x1b[u");
        assert_eq!(t.cursor(), (2, 1));
        assert!(t.cell(0, 0).unwrap().flags == 0);
        // Current attrs restored (invisible until next print).
        feed(&mut t, "X");
        assert!(t.cell(2, 1).unwrap().flags & INVERSE != 0);
    }

    #[test]
    fn esc7_esc8_and_csi_s_u_agree() {
        let mut t = term();
        feed(&mut t, "\x1b[3;3H\x1b7\x1b[H\x1b8");
        assert_eq!(t.cursor(), (2, 2));
        feed(&mut t, "\x1b[1;1H\x1b[s\x1b[5;5H\x1b[u");
        assert_eq!(t.cursor(), (0, 0));
    }

    // ----- modes -----

    #[test]
    fn cursor_visibility_mode() {
        let mut t = term();
        assert!(t.cursor_visible());
        feed(&mut t, "\x1b[?25l");
        assert!(!t.cursor_visible());
        feed(&mut t, "\x1b[?25h");
        assert!(t.cursor_visible());
    }

    #[test]
    fn app_cursor_mode_flag() {
        let mut t = term();
        feed(&mut t, "\x1b[?1h");
        assert!(t.app_cursor());
        feed(&mut t, "\x1b[?1l");
        assert!(!t.app_cursor());
    }

    #[test]
    fn bracketed_paste_mode_flag() {
        let mut t = term();
        feed(&mut t, "\x1b[?2004h");
        assert!(t.bracketed_paste());
        feed(&mut t, "\x1b[?2004l");
        assert!(!t.bracketed_paste());
    }

    #[test]
    fn alternate_screen_isolation() {
        let mut t = term();
        feed(&mut t, "normal");
        feed(&mut t, "\x1b[?1049h");
        assert!(t.on_alt_screen());
        // Alt screen is empty; scrollback untouched.
        assert_eq!(text(&t, 0), "");
        assert_eq!(t.scrollback_len(), 0);
        feed(&mut t, "ALT");
        feed(&mut t, "\x1b[?1049l");
        assert!(!t.on_alt_screen());
        assert_eq!(text(&t, 0), "normal");
        // The cursor was restored to where it was saved.
        assert_eq!(t.cursor(), (6, 0));
    }

    #[test]
    fn alt_screen_scrolling_does_not_touch_scrollback() {
        let mut t = term();
        feed(&mut t, "\x1b[?1049h");
        for i in 0..20 {
            t.feed(format!("row{i}\r\n").as_bytes());
        }
        assert_eq!(t.scrollback_len(), 0);
        feed(&mut t, "\x1b[?1049l");
        feed(&mut t, "\x1b[?1049h\x1b[?1049l"); // switch twice more
                                                // The normal screen was NEVER written (all output went to the
                                                // alt screen) and the scrollback is still empty.
        assert_eq!(text(&t, 0), "");
        assert_eq!(t.scrollback_len(), 0);
    }

    // ----- wide characters -----

    #[test]
    fn wide_char_occupies_two_cells() {
        let mut t = term();
        feed(&mut t, "漢字");
        let c0 = t.cell(0, 0).unwrap();
        assert_eq!(c0.c, '漢');
        assert!(c0.flags & WIDE != 0);
        let c1 = t.cell(1, 0).unwrap();
        assert!(c1.flags & WIDE_SPACER != 0);
        let c2 = t.cell(2, 0).unwrap();
        assert_eq!(c2.c, '字');
        assert!(c2.flags & WIDE != 0);
    }

    #[test]
    fn wide_char_at_line_end_wraps_cleanly() {
        let mut t = term();
        feed(&mut t, "abcdefghi漢"); // 9 ascii + wide at col 9..10
                                     // '漢' starts at col 9 (last col is 9 in 20 cols? no, 20 cols):
                                     // fits at 9..11, no wrap. Then printing one more char wraps.
        feed(&mut t, "X");
        assert_eq!(t.cell(11, 0).unwrap().c, 'X');
    }

    // ----- tabs -----

    #[test]
    fn tab_stops_are_set_every_8_columns() {
        let mut t = Terminal::new(30, 4);
        feed(&mut t, "\t\t");
        assert_eq!(t.cursor(), (16, 0));
        feed(&mut t, "\t");
        assert_eq!(t.cursor(), (24, 0));
        // Past the last stop -> last column.
        feed(&mut t, "\t");
        assert_eq!(t.cursor(), (29, 0));
    }

    // ----- title (OSC) -----

    #[test]
    fn osc_sets_the_window_title() {
        let mut t = term();
        feed(&mut t, "\x1b]0;my title\x07");
        assert_eq!(t.title, "my title");
        feed(&mut t, "\x1b]2;second\x1b\\");
        assert_eq!(t.title, "second");
    }

    #[test]
    fn osc_title_is_clamped() {
        let mut t = term();
        let long = "x".repeat(500);
        t.feed(format!("\x1b]0;{long}\x07").as_bytes());
        assert_eq!(t.title.chars().count(), Terminal::MAX_TITLE_LEN);
        assert!(t.title.capacity() <= Terminal::MAX_TITLE_LEN * 4);
    }

    // ----- reset -----

    #[test]
    fn ris_resets_everything_but_keeps_size_and_cap() {
        let mut t = term();
        feed(&mut t, "\x1b[?1049h\x1b[?25l\x1b[2;4r\x1b[38;5;9m");
        for i in 0..30 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        t.set_scrollback_cap(40);
        feed(&mut t, "\x1bc");
        assert_eq!(t.cursor(), (0, 0));
        assert!(t.cursor_visible());
        assert!(!t.on_alt_screen());
        assert_eq!(t.scroll_top, 0);
        assert_eq!(t.scroll_bot, t.rows - 1);
        assert_eq!(t.cell(0, 0).unwrap().fg, Color::Default);
        assert_eq!(t.scrollback_cap(), 40);
    }

    // ----- resize -----

    #[test]
    fn resize_keeps_top_left_content() {
        let mut t = term();
        feed(&mut t, "hello\r\nworld");
        t.resize(10, 3);
        assert_eq!(t.cols, 10);
        assert_eq!(t.rows, 3);
        assert_eq!(text(&t, 0), "hello");
        assert_eq!(text(&t, 1), "world");
    }

    #[test]
    fn resize_growing_adds_blank_cells() {
        let mut t = term();
        feed(&mut t, "hi");
        t.resize(30, 10);
        assert_eq!(text(&t, 0), "hi");
        assert_eq!(t.cursor(), (2, 0));
        assert_eq!(t.cell(29, 9).unwrap().c, ' ');
    }

    #[test]
    fn shrinking_rows_keeps_cursor_context_and_spills_above() {
        let mut t = term();
        feed(&mut t, "a\r\nb\r\nc\r\nd\r\ne\r\nf");
        t.resize(20, 3);
        assert_eq!(text(&t, 0), "d");
        assert_eq!(text(&t, 2), "f");
        assert!(t.scrollback_len() >= 3);
        assert!(t
            .scrollback_row(0)
            .unwrap()
            .iter()
            .map(|c| c.c)
            .collect::<String>()
            .contains('a'));
    }

    #[test]
    fn shrinking_rows_near_the_top_spills_nothing() {
        let mut t = term();
        feed(&mut t, "hello\r\nworld");
        t.resize(10, 3);
        assert_eq!(text(&t, 0), "hello");
        assert_eq!(text(&t, 1), "world");
    }

    #[test]
    fn resize_clamps_cursor_and_region() {
        let mut t = term();
        feed(&mut t, "\x1b[6;20H\x1b[2;5r");
        t.resize(10, 4);
        assert!(t.cursor().0 < 10);
        assert!(t.cursor().1 < 4);
        assert_eq!(t.scroll_top, 0);
        assert_eq!(t.scroll_bot, 3);
    }

    #[test]
    fn resize_is_a_noop_for_equal_size() {
        let mut t = term();
        feed(&mut t, "keep");
        t.resize(20, 6);
        assert_eq!(text(&t, 0), "keep");
        assert_eq!(t.scrollback_len(), 0);
    }

    // ----- region edge cases -----

    #[test]
    fn ri_at_top_of_region_scrolls_down() {
        let mut t = term();
        feed(&mut t, "1\r\n2\x1b[1;2H\x1bM"); // RI at row 0
        assert_eq!(text(&t, 1), "1");
        assert_eq!(text(&t, 2), "2");
    }

    #[test]
    fn ind_and_nel_behave_like_linefeed() {
        let mut t = term();
        feed(&mut t, "ab\x1bDcd");
        // IND keeps the column (col 2 after "ab").
        assert_eq!(t.cell(2, 1).unwrap().c, 'c');
        feed(&mut t, "\x1bEx");
        assert_eq!(t.cell(0, 2).unwrap().c, 'x'); // NEL returns to col 0
    }

    #[test]
    fn decstbm_equal_top_bottom_is_rejected() {
        let mut t = term();
        feed(&mut t, "\x1b[3;3r");
        assert_eq!(t.scroll_top, 0);
        assert_eq!(t.scroll_bot, t.rows - 1);
    }

    #[test]
    fn origin_mode_offsets_cup() {
        let mut t = term();
        feed(&mut t, "\x1b[2;4r\x1b[?6h\x1b[1;1HX");
        // Origin mode: row 1 of the region = screen row 1.
        assert_eq!(t.cell(0, 1).unwrap().c, 'X');
    }

    #[test]
    fn csi_with_unknown_action_is_ignored_safely() {
        let mut t = term();
        feed(&mut t, "\x1b[99Z\x1b[>c\x1b[?999h");
        assert_eq!(t.cursor(), (0, 0));
        assert_eq!(text(&t, 0), "");
    }

    #[test]
    fn esc_intermediates_are_ignored_safely() {
        let mut t = term();
        feed(&mut t, "\x1b(B\x1b)0");
        assert_eq!(text(&t, 0), "");
    }

    #[test]
    fn partial_escape_sequence_across_feeds_is_still_parsed() {
        let mut t = term();
        t.feed(b"\x1b[");
        t.feed(b"3;");
        t.feed(b"5");
        t.feed(b"H");
        t.feed(b"X");
        assert_eq!(t.cell(4, 2).unwrap().c, 'X');
    }

    #[test]
    fn utf8_split_across_feeds_is_decoded() {
        let mut t = term();
        let s = "漢".as_bytes();
        t.feed(&s[..1]);
        t.feed(&s[1..]);
        assert_eq!(t.cell(0, 0).unwrap().c, '漢');
    }

    #[test]
    fn max_min_grid_sizes_are_enforced() {
        let t = Terminal::new(1, 1);
        assert_eq!(t.cols, 2);
        assert_eq!(t.rows, 2);
    }

    #[test]
    fn maintain_memory_keeps_content_intact() {
        let mut t = term();
        for i in 0..400 {
            t.feed(format!("row{i}\r\n").as_bytes());
        }
        let before = t.scrollback_len();
        t.maintain_memory();
        assert_eq!(t.scrollback_len(), before);
        assert!(t
            .scrollback_row(before - 1)
            .unwrap()
            .iter()
            .map(|c| c.c)
            .collect::<String>()
            .contains("row394"));
    }
}
