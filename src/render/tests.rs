//! Pixel-level tests for the renderer primitives and the CPU
//! reference painter. Compiled only under `cfg(test)`.

use crate::font::{FontFace, FontMetrics};

use super::*;
use crate::vt::{Color, Terminal};

fn tiny_face() -> FontFace {
    // Minimal fake face is not possible with ab_glyph; tests use the
    // system mono font when present, otherwise they still validate
    // the pure pixel helpers below.
    crate::font::load_default_face("monospace", 14.0)
        .expect("system monospace font required for renderer tests")
}

#[test]
fn pack_and_unpack_round_trip() {
    assert_eq!(pack(0, 0, 0), 0x000000);
    assert_eq!(pack(255, 255, 255), 0xFFFFFF);
    assert_eq!(pack(0x12, 0x34, 0x56), 0x123456);
    assert_eq!(unpack(pack(0x12, 0x34, 0x56)), [0x12, 0x34, 0x56]);
}

#[test]
fn resolve_maps_indexed_and_rgb() {
    let mut ansi = [[0u8; 3]; 16];
    ansi[1] = [255, 0, 0];
    let p = Palette {
        ansi,
        foreground: [200, 200, 200],
        background: [10, 20, 30],
        cursor: [255, 255, 0],
    };
    assert_eq!(resolve(Color::Indexed(1), p.foreground, &p), [255, 0, 0]);
    assert_eq!(resolve(Color::Default, p.foreground, &p), [200, 200, 200]);
    assert_eq!(resolve(Color::Rgb([1, 2, 3]), p.foreground, &p), [1, 2, 3]);
    // 42 from the cube: [0, 215, 135].
    assert_eq!(resolve(Color::Indexed(42), p.foreground, &p), [0, 215, 135]);
}

#[test]
fn blend_interpolates_correctly() {
    assert_eq!(blend([255, 0, 0], [0, 0, 255], 0.0), [0, 0, 255]);
    assert_eq!(blend([255, 0, 0], [0, 0, 255], 1.0), [255, 0, 0]);
    let mid = blend([0, 0, 0], [255, 255, 255], 0.5);
    assert_eq!(mid, [128, 128, 128]);
}

#[test]
fn fill_rect_clips_to_buffer_bounds() {
    let mut buf = vec![0u32; 10 * 10];
    Canvas::new(&mut buf, 10, 10).fill_rect(-5, -5, 20, 20, pack(9, 9, 9));
    // Only the visible part is filled.
    assert_eq!(buf[0], pack(9, 9, 9));
    Canvas::new(&mut buf, 10, 10).fill_rect(8, 0, 10, 1, pack(1, 2, 3));
    assert_eq!(buf[9], pack(1, 2, 3));
    // Off-screen rect must not panic.
    Canvas::new(&mut buf, 10, 10).fill_rect(100, 100, 5, 5, 0);
    Canvas::new(&mut buf, 10, 10).fill_rect(-1, -1, 0, 0, 0);
}

#[test]
fn fill_rect_fills_rows_not_columns_only() {
    let mut buf = vec![0u32; 4 * 4];
    Canvas::new(&mut buf, 4, 4).fill_rect(1, 1, 2, 2, pack(7, 7, 7));
    let p = pack(7, 7, 7);
    assert_eq!((buf[5], buf[6], buf[9], buf[10]), (p, p, p, p));
    assert_ne!(buf[0], p);
}

#[test]
fn blit_glyph_blends_coverage() {
    let g = crate::font::GlyphBitmap {
        width: 2,
        height: 2,
        coverage: vec![255, 0, 128, 0],
        left: 0,
        top: 0,
    };
    let mut buf = vec![pack(0, 0, 0); 4 * 4];
    Canvas::new(&mut buf, 4, 4).blit_glyph(&g, 0, 0, [255, 255, 255]);
    assert_eq!(buf[0], pack(255, 255, 255)); // full coverage
    assert_eq!(buf[1], pack(0, 0, 0)); // zero coverage untouched
    let mid = unpack(buf[4]);
    assert_eq!(mid[0], 128); // 50% coverage blends halfway
}

#[test]
fn blit_glyph_clips_off_screen() {
    let g = crate::font::GlyphBitmap {
        width: 3,
        height: 3,
        coverage: vec![255; 9],
        left: 0,
        top: 0,
    };
    let mut buf = vec![0u32; 4 * 4];
    Canvas::new(&mut buf, 4, 4).blit_glyph(&g, 2, 2, [255, 0, 0]);
    assert_eq!(buf[2 * 4 + 2], pack(255, 0, 0));
    assert_eq!(buf[0], 0);
    // Fully off-screen must not panic.
    Canvas::new(&mut buf, 4, 4).blit_glyph(&g, 10, 10, [255, 0, 0]);
}

#[test]
fn underline_draws_at_cell_bottom() {
    let mut buf = vec![0u32; 4 * 8];
    Canvas::new(&mut buf, 4, 8).draw_underline(0, 0, 4, 8, [10, 20, 30]);
    // line_y = lh-2 = 6; index = y*w + x = 6*4 + 0.
    assert_eq!(buf[6 * 4], pack(10, 20, 30));
}

#[test]
fn paint_clears_with_theme_background() {
    let t = Terminal::new(4, 2);
    let mut ansi = [[0u8; 3]; 16];
    ansi[0] = [1, 2, 3];
    let p = Palette {
        ansi,
        foreground: [200, 200, 200],
        background: [10, 10, 10],
        cursor: [255, 255, 255],
    };
    let mut face = tiny_face();
    let w = face.metrics.cell_width as usize * 4 + 8;
    let h = face.metrics.line_height as usize * 2 + 8;
    let mut buf = vec![0u32; w * h];
    paint(&mut Canvas::new(&mut buf, w, h), &t, 0, &p, &mut face, None);
    // Bottom padding row stays theme background.
    let last = buf[buf.len() - 1];
    assert_eq!(unpack(last), [10, 10, 10]);
}

#[test]
fn paint_draws_text_in_theme_foreground() {
    let mut t = Terminal::new(10, 2);
    t.feed(b"ab");
    let ansi = [[0u8; 3]; 16];
    let p = Palette {
        ansi,
        foreground: [255, 128, 0],
        background: [0, 0, 0],
        cursor: [255, 255, 255],
    };
    let mut face = tiny_face();
    let w = face.metrics.cell_width as usize * 10;
    let h = face.metrics.line_height as usize * 2;
    let mut buf = vec![0u32; w * h];
    paint(&mut Canvas::new(&mut buf, w, h), &t, 0, &p, &mut face, None);
    // Some pixels inside the 'a' cell must be foreground-tinted.
    let cw = face.metrics.cell_width as usize;
    let lh = face.metrics.line_height as usize;
    let tinted = (0..cw)
        .flat_map(|x| (0..lh).map(move |y| (x, y)))
        .any(|(x, y)| unpack(buf[y * w + x]) != [0, 0, 0]);
    assert!(tinted, "the 'a' cell must contain glyph ink");
    // An empty cell far right stays background.
    assert_eq!(unpack(buf[(w - 1) + lh * 2 - 1]), [0, 0, 0]);
}

#[test]
fn paint_inverse_swaps_colors() {
    let mut t = Terminal::new(8, 2);
    t.feed(b"\x1b[7mX");
    let ansi = [[0u8; 3]; 16];
    let p = Palette {
        ansi,
        foreground: [255, 255, 255],
        background: [5, 5, 5],
        cursor: [128, 128, 128],
    };
    let mut face = tiny_face();
    let w = face.metrics.cell_width as usize * 8;
    let h = face.metrics.line_height as usize * 2;
    let mut buf = vec![0u32; w * h];
    paint(&mut Canvas::new(&mut buf, w, h), &t, 0, &p, &mut face, None);
    // The cell behind 'X' must be foreground-colored (inverse bg).
    let cw = face.metrics.cell_width as usize;
    let corner = buf[cw - 1];
    assert_eq!(unpack(corner), [255, 255, 255]);
}

#[test]
fn paint_cursor_block_is_visible() {
    let mut t = Terminal::new(8, 2);
    let ansi = [[0u8; 3]; 16];
    let p = Palette {
        ansi,
        foreground: [255, 255, 255],
        background: [0, 0, 0],
        cursor: [255, 0, 0],
    };
    let mut face = tiny_face();
    let w = face.metrics.cell_width as usize * 8;
    let h = face.metrics.line_height as usize * 2;
    let mut buf = vec![0u32; w * h];
    paint(&mut Canvas::new(&mut buf, w, h), &t, 0, &p, &mut face, None);
    // Top-left cell = cursor block -> red.
    assert_eq!(
        unpack(buf[face.metrics.cell_width as usize / 2]),
        [255, 0, 0]
    );
    // Cursor hidden -> no red block.
    t.feed(b"\x1b[?25l");
    let mut buf2 = vec![0u32; w * h];
    paint(
        &mut Canvas::new(&mut buf2, w, h),
        &t,
        0,
        &p,
        &mut face,
        None,
    );
    assert_ne!(
        unpack(buf2[face.metrics.cell_width as usize / 2]),
        [255, 0, 0]
    );
}

#[test]
fn paint_scrolled_view_shows_scrollback_rows() {
    let mut t = Terminal::new(8, 2);
    t.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive"); // 5 lines, 2-row screen
    let sb = t.scrollback_len();
    assert!(sb >= 3);
    let ansi = [[0u8; 3]; 16];
    let p = Palette {
        ansi,
        foreground: [255, 255, 255],
        background: [0, 0, 0],
        cursor: [128, 128, 128],
    };
    let mut face = tiny_face();
    let w = face.metrics.cell_width as usize * 8;
    let h = face.metrics.line_height as usize * 2;
    // Offset 0: top line is "three".
    let mut live = vec![0u32; w * h];
    paint(
        &mut Canvas::new(&mut live, w, h),
        &t,
        0,
        &p,
        &mut face,
        None,
    );
    // Offset 2: top visible line is "one".
    let mut scrolled = vec![0u32; w * h];
    paint(
        &mut Canvas::new(&mut scrolled, w, h),
        &t,
        2,
        &p,
        &mut face,
        None,
    );
    // The two frames must differ somewhere.
    assert_ne!(live, scrolled, "scrolling must change the frame");
}

#[test]
fn metrics_used_by_paint_are_consistent() {
    // Cell sizes must be strictly positive so paint never divides
    // by zero or loops forever.
    let face = tiny_face();
    let m: FontMetrics = face.metrics;
    assert!(m.cell_width >= 1 && m.line_height >= 1);
}

#[test]
fn selection_contains_orders_its_endpoints() {
    // Linear (text-flow) selection: start (5,1) -> end (2,3) means
    // row 1 from col 5 to the edge, row 2 fully, row 3 up to col 2.
    let s = Selection {
        start: (5, 1),
        end: (2, 3),
    };
    assert!(s.contains(5, 1));
    assert!(s.contains(2, 3));
    assert!(s.contains(0, 2)); // full middle row
    assert!(s.contains(6, 1)); // same row as start, after it
    assert!(!s.contains(4, 1)); // before the start column
    assert!(s.contains(1, 3)); // up to the end column
    assert!(!s.contains(3, 3)); // past the end column
    assert!(!s.contains(2, 4)); // below the end row
    assert!(!s.contains(2, 0)); // above the start row
}

#[test]
fn selection_on_one_row_is_a_range() {
    let s = Selection {
        start: (1, 0),
        end: (3, 0),
    };
    assert!(s.contains(1, 0));
    assert!(s.contains(2, 0));
    assert!(s.contains(3, 0));
    assert!(!s.contains(0, 0));
    assert!(!s.contains(4, 0));
    assert!(!s.contains(2, 1));
}

#[test]
fn selected_text_extracts_the_range() {
    // Wide enough that neither line wraps.
    let mut t = Terminal::new(20, 4);
    t.feed(b"hello world\r\nsecond line");
    let s = Selection {
        start: (0, 0),
        end: (4, 0),
    };
    assert_eq!(s.text(&t), "hello");
    let s = Selection {
        start: (0, 0),
        end: (10, 1),
    };
    assert_eq!(s.text(&t), "hello world\nsecond line");
}

#[test]
fn paint_highlights_selected_cells() {
    let mut t = Terminal::new(6, 2);
    t.feed(b"abcdef");
    let ansi = [[0u8; 3]; 16];
    let p = Palette {
        ansi,
        foreground: [255, 255, 255],
        background: [0, 0, 0],
        cursor: [0, 255, 0],
    };
    let mut face = tiny_face();
    let cw = face.metrics.cell_width as usize;
    let lh = face.metrics.line_height as usize;
    let w = cw * 6;
    let h = lh * 2;
    let mut with_sel = vec![0u32; w * h];
    let sel = Selection {
        start: (0, 0),
        end: (2, 0),
    };
    paint(
        &mut Canvas::new(&mut with_sel, w, h),
        &t,
        0,
        &p,
        &mut face,
        Some(&sel),
    );
    let mut without = vec![0u32; w * h];
    paint(
        &mut Canvas::new(&mut without, w, h),
        &t,
        0,
        &p,
        &mut face,
        None,
    );
    assert_ne!(with_sel, without);
    // The bottom of a selected cell (no glyph ink there) must be
    // highlighted, not plain background.
    assert_ne!(with_sel[lh - 1], without[lh - 1]);
}

// ---- regression tests: palettless themes must never panic ----

fn term_colors_with_palette(palette: Vec<crate::theme::Rgba>) -> crate::theme::TerminalColors {
    crate::theme::TerminalColors {
        background: crate::theme::Rgba::opaque("#1A1B26"),
        foreground: crate::theme::Rgba::opaque("#C0CAF5"),
        cursor: crate::theme::Rgba::opaque("#C0CAF5"),
        palette,
    }
}

#[test]
fn from_terminal_colors_never_panics_on_an_empty_palette() {
    // Exact regression for the reported startup crash:
    // "index out of bounds: the len is 0 but the index is 0".
    let colors = term_colors_with_palette(Vec::new());
    let p = Palette::from_terminal_colors(&colors);
    assert_eq!(p.ansi.len(), 16);
    // Missing slots resolve to the standard xterm base colors.
    assert_eq!(p.ansi[0], [0x00, 0x00, 0x00]); // black
    assert_eq!(p.ansi[1], [0xCD, 0x00, 0x00]); // red
    assert_eq!(p.ansi[2], [0x00, 0xCD, 0x00]); // green
    assert_eq!(p.ansi[4], [0x00, 0x00, 0xEE]); // blue
    assert_eq!(p.ansi[8], [0x7F, 0x7F, 0x7F]); // bright black
    assert_eq!(p.ansi[15], [0xFF, 0xFF, 0xFF]); // bright white
                                                // fg/bg/cursor still come through untouched.
    assert_eq!(p.foreground, [0xC0, 0xCA, 0xF5]);
    assert_eq!(p.background, [0x1A, 0x1B, 0x26]);
}

#[test]
fn from_terminal_colors_fills_a_partial_palette() {
    // A theme with a single palette entry: slot 0 is custom, the
    // rest are the standard base.
    let colors = term_colors_with_palette(vec![crate::theme::Rgba::opaque("#ABCDEF")]);
    let p = Palette::from_terminal_colors(&colors);
    assert_eq!(p.ansi[0], [0xAB, 0xCD, 0xEF]);
    assert_eq!(
        p.ansi[1],
        [0xCD, 0x00, 0x00],
        "slot 1 falls back to standard red"
    );
    assert_eq!(
        p.ansi[15],
        [0xFF, 0xFF, 0xFF],
        "slot 15 falls back to standard white"
    );
}

#[test]
fn from_terminal_colors_converts_a_full_palette_exactly() {
    let colors = term_colors_with_palette(
        crate::theme::STANDARD_ANSI_16
            .iter()
            .map(|h| crate::theme::Rgba::opaque(h))
            .collect(),
    );
    let p = Palette::from_terminal_colors(&colors);
    for (i, hex) in crate::theme::STANDARD_ANSI_16.iter().enumerate() {
        let expected = crate::theme::Rgba::opaque(hex);
        let want = [
            (expected.r * 255.0).round() as u8,
            (expected.g * 255.0).round() as u8,
            (expected.b * 255.0).round() as u8,
        ];
        assert_eq!(p.ansi[i], want, "slot {i}");
    }
}

#[test]
fn every_builtin_theme_converts_without_panicking() {
    // The exact startup path of gui.rs::palette_from_settings run
    // for every built-in theme: the conversion itself must never
    // panic, whatever the registry contains.
    let reg = crate::theme::Registry::builtin();
    for name in reg.names() {
        let colors = crate::theme::terminal_colors(&name, false, "off");
        let p = Palette::from_terminal_colors(&colors);
        assert_eq!(p.ansi.len(), 16, "{name}");
    }
}
