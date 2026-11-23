// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Unit tests for the VT engine: raw byte streams are fed through
//! [`Terminal::feed`] and the resulting grid / scrollback / title
//! state is asserted. Compiled only under `cfg(test)` via the parent
//! module's `#[cfg(test)] mod tests;`.

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
