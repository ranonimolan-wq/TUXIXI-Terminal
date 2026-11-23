// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! VTE protocol implementation: the `Perform` adapter that turns
//! parsed escape sequences into grid operations on [`Terminal`].
//! Parsing helpers and the memory report live in the sibling `model`
//! module.

use vte::{Params, Perform};

use super::model::{Cell, Terminal, WIDE, WIDE_SPACER};

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
