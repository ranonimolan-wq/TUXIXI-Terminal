// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Behavioral tests for the terminal engine: raw byte streams are fed
//! through [`Terminal::feed`] and the resulting grid, scrollback,
//! title, mode and query-response state is asserted. Compiled only
//! under `cfg(test)` via the parent module's `#[cfg(test)] mod tests;`.

use super::*;

fn term() -> Terminal {
    Terminal::new(20, 6)
}

fn feed(t: &mut Terminal, s: &str) {
    t.feed(s.as_bytes());
}

/// Render one visible row of the live screen as text.
fn text(t: &mut Terminal, y: u16) -> String {
    let mut grid = Vec::new();
    t.copy_visible(0, &mut grid);
    let cols = t.cols as usize;
    grid[y as usize * cols..(y as usize + 1) * cols]
        .iter()
        .map(|c| c.c)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Drain and collect the PTY-bound responses the engine produced.
fn responses(t: &mut Terminal) -> Vec<Vec<u8>> {
    t.take_events()
        .into_iter()
        .filter_map(|ev| match ev {
            TermEvent::PtyWrite(bytes) => Some(bytes),
            _ => None,
        })
        .collect()
}

// ----- basics -----

#[test]
fn plain_text_lands_on_the_grid() {
    let mut t = term();
    feed(&mut t, "hello world");
    assert_eq!(text(&mut t, 0), "hello world");
    assert_eq!(t.cursor(), (11, 0));
}

#[test]
fn cr_moves_without_linefeed_and_lf_moves_without_carriage() {
    let mut t = term();
    feed(&mut t, "abc\rxy");
    assert_eq!(text(&mut t, 0), "xyc");
    feed(&mut t, "\nZ");
    assert_eq!(text(&mut t, 1), "  Z");
    // CR is NOT implicit in LF (ONLCR happens in the line discipline):
    // the column is kept.
    assert_eq!(t.cursor(), (3, 1));
}

#[test]
fn overwrite_replaces_cells_without_erasing_the_rest() {
    let mut t = term();
    feed(&mut t, "abcdef");
    feed(&mut t, "\rXYZ");
    assert_eq!(text(&mut t, 0), "XYZdef");
}

#[test]
fn bottom_line_scrolls_the_grid_into_scrollback() {
    let mut t = term();
    for i in 0..8u16 {
        feed(&mut t, &format!("row{i}\r\n"));
    }
    assert_eq!(text(&mut t, 0), "row3");
    assert!(t.scrollback_len() >= 1);
}

// ----- erase / cursor operations (shell redraw surface) -----

#[test]
fn el_clears_from_the_cursor_and_the_full_line() {
    let mut t = term();
    feed(&mut t, "abcdefgh\r\x1b[4CX");
    assert_eq!(text(&mut t, 0), "abcdXfgh");
    // EL 0 after CR wipes the whole line.
    feed(&mut t, "\r\x1b[K");
    assert_eq!(text(&mut t, 0), "");
}

#[test]
fn ech_erases_n_characters_in_place() {
    let mut t = term();
    feed(&mut t, "abcdef\r\x1b[3X");
    assert_eq!(text(&mut t, 0), "   def");
}

#[test]
fn cursor_positioning_matches_the_xterm_convention() {
    let mut t = term();
    feed(&mut t, "\x1b[2;3HX");
    assert_eq!(text(&mut t, 1), "  X");
    feed(&mut t, "\x1b[5D");
    // CUB clamps at column 0.
    assert_eq!(t.cursor(), (0, 1));
    feed(&mut t, "\x1b[2G");
    assert_eq!(t.cursor(), (1, 1));
}

#[test]
fn cpr_reports_the_real_cursor_position() {
    // Regression: the engine must answer DSR (CSI 6n) so shells and
    // line editors can resynchronize their cursor model.
    let mut t = term();
    feed(&mut t, "abc\r\nxyz");
    assert_eq!(t.cursor(), (3, 1));
    feed(&mut t, "\x1b[6n");
    let out = responses(&mut t);
    assert_eq!(out.len(), 1, "DSR must produce exactly one response");
    assert_eq!(String::from_utf8_lossy(&out[0]), "\x1b[2;4R");
}

// ----- attributes / colors -----

#[test]
fn sgr_colors_and_flags_reach_the_cells() {
    let mut t = term();
    feed(&mut t, "\x1b[1;31mR\x1b[0mg");
    let mut grid = Vec::new();
    t.copy_visible(0, &mut grid);
    assert_eq!(grid[0].fg, Color::Indexed(1));
    assert!(grid[0].flags & BOLD != 0);
    assert_eq!(grid[1].fg, Color::Default);
    assert_eq!(grid[1].flags & BOLD, 0);
}

#[test]
fn truecolor_and_extended_indexed_colors_are_kept() {
    let mut t = term();
    feed(&mut t, "\x1b[38;5;196ma\x1b[38;2;10;20;30mb");
    let mut grid = Vec::new();
    t.copy_visible(0, &mut grid);
    assert_eq!(grid[0].fg, Color::Indexed(196));
    assert_eq!(grid[1].fg, Color::Rgb([10, 20, 30]));
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

// ----- modes -----

#[test]
fn bracketed_paste_mode_tracks_decset_2004() {
    let mut t = term();
    assert!(!t.bracketed_paste());
    feed(&mut t, "\x1b[?2004h");
    assert!(t.bracketed_paste());
    feed(&mut t, "\x1b[?2004l");
    assert!(!t.bracketed_paste());
}

#[test]
fn app_cursor_mode_tracks_decset_1() {
    let mut t = term();
    assert!(!t.app_cursor());
    feed(&mut t, "\x1b[?1h");
    assert!(t.app_cursor());
    feed(&mut t, "\x1b[?1l");
    assert!(!t.app_cursor());
}

#[test]
fn alt_screen_swaps_buffers_and_scrollback_waits() {
    let mut t = term();
    feed(&mut t, "normal\r\ncontent");
    feed(&mut t, "\x1b[?1049h");
    assert!(t.on_alt_screen());
    // The alt screen starts blank at the restored cursor position.
    assert_eq!(text(&mut t, 0), "");
    feed(&mut t, "\x1b[Haltscreen");
    assert_eq!(text(&mut t, 0), "altscreen");
    feed(&mut t, "\x1b[?1049l");
    assert!(!t.on_alt_screen());
    assert_eq!(text(&mut t, 0), "normal");
}

#[test]
fn cursor_can_be_hidden_by_decset_25() {
    let mut t = term();
    assert!(t.cursor_visible());
    feed(&mut t, "\x1b[?25l");
    assert!(!t.cursor_visible());
    feed(&mut t, "\x1b[?25h");
    assert!(t.cursor_visible());
}

// ----- wide characters -----

#[test]
fn wide_char_occupies_two_cells_with_a_spacer() {
    let mut t = term();
    feed(&mut t, "漢a");
    let mut grid = Vec::new();
    t.copy_visible(0, &mut grid);
    assert_eq!(grid[0].c, '漢');
    assert!(grid[0].flags & WIDE != 0);
    assert!(grid[1].flags & WIDE_SPACER != 0);
    assert_eq!(grid[2].c, 'a');
    assert_eq!(t.cursor(), (3, 0));
}

// ----- scrolling -----

#[test]
fn scroll_region_lims_scroll_movement() {
    let mut t = term();
    feed(&mut t, "\x1b[2;4r");
    feed(&mut t, "\x1b[2;1Hone\r\ntwo\r\nthree\r\nfour");
    // The CRLF after `three` scrolls the region [2,4] (1-based), so
    // `four` is written on the region's bottom row.
    assert_eq!(text(&mut t, 1), "two");
    assert_eq!(text(&mut t, 2), "three");
    assert_eq!(text(&mut t, 3), "four");
}

#[test]
fn scrollback_cap_is_enforced() {
    let mut t = term();
    t.set_scrollback_cap(25);
    for i in 0..1000u16 {
        feed(&mut t, &format!("line{i}\r\n"));
    }
    assert!(t.scrollback_len() <= 25);
}

#[test]
fn scrolled_viewport_shows_scrollback_rows() {
    let mut t = term();
    for i in 0..10u16 {
        feed(&mut t, &format!("line{i}\r\n"));
    }
    // Scroll back one row: the top visible row becomes line0's neighbor.
    let live = text(&mut t, 0);
    let mut grid = Vec::new();
    t.copy_visible(1, &mut grid);
    let back = grid[0..t.cols as usize]
        .iter()
        .map(|c| c.c)
        .collect::<String>()
        .trim_end()
        .to_string();
    assert_ne!(back, live.clone());
    assert!(back.starts_with("line"), "{back:?}");
    // Live grid is untouched by viewing scrollback.
    assert_eq!(text(&mut t, 0), live);
}

// ----- title / memory / misc -----

#[test]
fn osc_title_is_stored_and_resettable() {
    let mut t = term();
    feed(&mut t, "\x1b]0;my window title\x07");
    assert_eq!(t.title, "my window title");
    feed(&mut t, "\x1b]0;\x07");
    assert_eq!(t.title, "");
}

#[test]
fn osc_title_is_clamped_to_the_byte_limit() {
    let mut t = term();
    let long = "x".repeat(500);
    t.feed(format!("\x1b]0;{long}\x07").as_bytes());
    assert!(t.title.len() <= Terminal::MAX_TITLE_LEN);
}

#[test]
fn memory_report_reflects_bounds() {
    let t = term();
    let r = t.memory_report();
    assert_eq!(r.live_cells, 20 * 6);
    assert!(r.scrollback_cap >= r.scrollback_rows);
    assert!(r.cell_bytes > 0);
}

#[test]
fn resize_repositions_the_grid() {
    let mut t = term();
    feed(&mut t, "abcdefghij");
    t.resize(30, 10);
    assert_eq!(t.cols, 30);
    assert_eq!(t.rows, 10);
    assert_eq!(text(&mut t, 0), "abcdefghij");
}

#[test]
fn feed_is_noop_for_empty_input() {
    let mut t = term();
    t.feed(b"");
    assert_eq!(responses(&mut t).len(), 0);
    assert_eq!(text(&mut t, 0), "");
}
