// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Terminal grid model: the character cell grid, color references,
//! saved-cursor state, the core [`Terminal`] operations (resize,
//! scrollback ring, tab stops, SGR handling, memory report) and the
//! extended-color (SGR 38/48/58) parsers its SGR path calls.

use crate::mem::BoundedDeque;
use vte::Params;

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
    pub(crate) col: u16,
    pub(crate) row: u16,
    origin: bool,
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) flags: u16,
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

    pub(crate) col: u16,
    pub(crate) row: u16,
    pub(crate) origin_mode: bool,
    autowrap: bool,
    pub(crate) pending_wrap: bool,
    app_cursor: bool,
    bracketed_paste: bool,
    cursor_visible: bool,

    pub(crate) scroll_top: u16,
    pub(crate) scroll_bot: u16,
    saved: SavedCursor,

    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) flags: u16,

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

    pub(crate) fn active_grid_mut(&mut self) -> &mut [Cell] {
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

    pub(crate) fn cell_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
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

    pub(crate) fn clamp_to_region_row(&self, y: u16) -> u16 {
        y.clamp(self.scroll_top, self.scroll_bot)
    }

    /// Scroll the region [top, bot] up by n rows (content moves up).
    pub(crate) fn scroll_up(&mut self, top: u16, bot: u16, n: u16) {
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
    pub(crate) fn scroll_down(&mut self, top: u16, bot: u16, n: u16) {
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
    pub(crate) fn linefeed(&mut self) {
        if self.row == self.scroll_bot {
            self.scroll_up(self.scroll_top, self.scroll_bot, 1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    pub(crate) fn carriage_return(&mut self) {
        self.col = 0;
        self.pending_wrap = false;
    }

    pub(crate) fn backspace(&mut self) {
        self.col = self.col.saturating_sub(1);
        self.pending_wrap = false;
    }

    pub(crate) fn advance_tab(&mut self) {
        let mut x = self.col as usize + 1;
        while x < self.tabstops.len() && !self.tabstops[x] {
            x += 1;
        }
        self.col = (x as u16).min(self.cols - 1);
        self.pending_wrap = false;
    }

    /// Wrap-aware cursor advance after printing a glyph of `width` cells.
    pub(crate) fn advance(&mut self, width: u16) {
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

    pub(crate) fn clear_row_range(&mut self, y: u16, x0: u16, x1: u16) {
        let bg = self.bg;
        for x in x0..=x1.min(self.cols - 1) {
            if let Some(cell) = self.cell_mut(x, y) {
                *cell = Cell::blank_with_bg(bg);
            }
        }
    }

    pub(crate) fn erase_all(&mut self) {
        let cols = self.cols;
        for y in 0..self.rows {
            self.clear_row_range(y, 0, cols - 1);
        }
    }

    // -------------------------------------------------------------
    // vte::Perform implementation.
    // -------------------------------------------------------------

    pub(crate) fn param(&self, params: &Params, index: usize, default: u16) -> u16 {
        let collected: Vec<&[u16]> = params.iter().collect();
        collected
            .get(index)
            .and_then(|s| s.first().copied())
            .filter(|&v| v != 0)
            .unwrap_or(default)
    }

    pub(crate) fn apply_sgr(&mut self, params: &Params) {
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

    pub(crate) fn set_mode(&mut self, private: bool, params: &Params, enable: bool) {
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

    pub(crate) fn save_cursor(&mut self) {
        self.saved = SavedCursor {
            col: self.col,
            row: self.row,
            origin: self.origin_mode,
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
        };
    }

    pub(crate) fn restore_cursor(&mut self) {
        let s = self.saved;
        self.col = s.col.min(self.cols - 1);
        self.row = self.clamp_to_region_row(s.row).min(self.rows - 1);
        self.origin_mode = s.origin;
        self.fg = s.fg;
        self.bg = s.bg;
        self.flags = s.flags;
        self.pending_wrap = false;
    }

    pub(crate) fn full_reset(&mut self) {
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
