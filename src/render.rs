//! Terminal drawing logic shared by all renderers: the [`Palette`],
//! [`Selection`] and the pure color helpers ([`resolve`], [`blend`],
//! [`pack`], [`fill_rect`], [`blit_glyph`]).
//!
//! The actual GUI renders on the GPU (`crate::gpu`, OpenGL 3.3); the
//! [`paint`] function in this module is the CPU reference renderer
//! that mirrors the exact same cell semantics pixel by pixel — it
//! doubles as the oracle for the renderer tests. The whole frame is a
//! few MB of plain arithmetic, so tests can assert on real pixels.

use crate::font::{FontFace, STYLE_BOLD, STYLE_ITALIC};
use crate::vt::{Cell, Color, Terminal, BOLD, DIM, INVERSE, ITALIC, UNDERLINE, WIDE_SPACER};

/// Resolved theme colors handed to the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// 16 base colors (0-15).
    pub ansi: [[u8; 3]; 16],
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub cursor: [u8; 3],
}

impl Palette {
    /// Build from the theme pipeline (`theme::terminal_colors`).
    ///
    /// A theme may ship fewer than 16 palette entries (or none at all),
    /// so missing slots fall back to the standard xterm colors instead
    /// of panicking. Regression: a palettless user theme overriding a
    /// built-in used to crash here at startup with
    /// "index out of bounds: the len is 0 but the index is 0".
    pub fn from_terminal_colors(colors: &crate::theme::TerminalColors) -> Self {
        let mut ansi = [[0u8; 3]; 16];
        for (i, slot) in ansi.iter_mut().enumerate() {
            let c = colors
                .palette
                .get(i)
                .copied()
                .unwrap_or_else(|| crate::theme::standard_ansi(i));
            *slot = [
                (c.r * 255.0).round() as u8,
                (c.g * 255.0).round() as u8,
                (c.b * 255.0).round() as u8,
            ];
        }
        Self {
            ansi,
            foreground: rgba_bytes(colors.foreground),
            background: rgba_bytes(colors.background),
            cursor: rgba_bytes(colors.cursor),
        }
    }
}

fn rgba_bytes(c: crate::theme::Rgba) -> [u8; 3] {
    [
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    ]
}

/// Pack RGB into the CPU reference renderer's opaque 0x00RRGGBB pixel
/// format.
pub fn pack(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// Unpack a framebuffer pixel back into RGB (test helper + blending).
pub fn unpack(px: u32) -> [u8; 3] {
    [(px >> 16) as u8, (px >> 8) as u8, px as u8]
}

/// Resolve a grid color reference to concrete RGB.
pub fn resolve(c: Color, default: [u8; 3], palette: &Palette) -> [u8; 3] {
    match c {
        Color::Default => default,
        Color::Indexed(i) if (i as usize) < 16 => palette.ansi[i as usize],
        Color::Indexed(i) => match crate::vt::indexed_color(i) {
            Color::Rgb(rgb) => rgb,
            _ => default,
        },
        Color::Rgb(rgb) => rgb,
    }
}

/// Blend two colors (0.0..=1.0 coverage).
pub fn blend(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 * t + y as f32 * (1.0 - t)).round() as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

/// Fill a rect with a solid packed color, clipped to the buffer.
#[allow(clippy::too_many_arguments)]
pub fn fill_rect(
    buf: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    px: u32,
) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = ((x as i64 + w as i64) as usize).min(width);
    let y1 = ((y as i64 + h as i64) as usize).min(height);
    if x0 >= x1 || y0 >= y1 || width == 0 {
        return;
    }
    for row in y0..y1 {
        let start = row * width + x0;
        let end = row * width + x1;
        buf[start..end].fill(px);
    }
}

/// Blit a glyph coverage bitmap with per-pixel alpha blending.
#[allow(clippy::too_many_arguments)]
pub fn blit_glyph(
    buf: &mut [u32],
    width: usize,
    height: usize,
    glyph: &crate::font::GlyphBitmap,
    dest_x: i32,
    dest_y: i32,
    fg: [u8; 3],
    bg: [u8; 3],
) {
    for gy in 0..glyph.height as i32 {
        let py = dest_y + gy;
        if py < 0 || py as usize >= height {
            continue;
        }
        for gx in 0..glyph.width as i32 {
            let pxx = dest_x + gx;
            if pxx < 0 || pxx as usize >= width {
                continue;
            }
            let cov = glyph.coverage[gy as usize * glyph.width + gx as usize];
            if cov == 0 {
                continue;
            }
            let idx = py as usize * width + pxx as usize;
            let base = unpack(buf[idx]);
            let color = if cov == 255 {
                fg
            } else {
                blend(fg, base, cov as f32 / 255.0)
            };
            buf[idx] = pack(color[0], color[1], color[2]);
        }
    }
    let _ = bg;
}

/// Draw a horizontal underline at the bottom of a cell.
#[allow(clippy::too_many_arguments)]
pub fn draw_underline(
    buf: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    cw: u32,
    lh: u32,
    fg: [u8; 3],
) {
    let line_y = y + lh as i32 - 2;
    fill_rect(
        buf,
        width,
        height,
        x,
        line_y,
        cw,
        1,
        pack(fg[0], fg[1], fg[2]),
    );
}

/// A rectangular cell selection on the live screen (mouse drag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: (u16, u16),
    pub end: (u16, u16),
}

impl Selection {
    /// Order the two endpoints, then test membership.
    pub fn bounds(&self) -> ((u16, u16), (u16, u16)) {
        let ((ax, ay), (bx, by)) = (self.start, self.end);
        if (ay, ax) <= (by, bx) {
            ((ax, ay), (bx, by))
        } else {
            ((bx, by), (ax, ay))
        }
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        let ((x0, y0), (x1, y1)) = self.bounds();
        if y0 == y1 {
            y == y0 && x >= x0 && x <= x1
        } else {
            (y == y0 && x >= x0) || (y == y1 && x <= x1) || (y > y0 && y < y1)
        }
    }

    /// Collect the selected text from the live grid.
    pub fn text(&self, term: &Terminal) -> String {
        let ((x0, y0), (x1, y1)) = self.bounds();
        let mut out = String::new();
        for y in y0..=y1.min(term.rows - 1) {
            let cols = term.cols;
            let lo = if y == y0 { x0 } else { 0 };
            let hi = if y == y1 { x1.min(cols - 1) } else { cols - 1 };
            let mut line = String::new();
            let mut prev_wide = false;
            for x in lo..=hi {
                if let Some(c) = term.cell(x, y) {
                    if c.flags & crate::vt::WIDE_SPACER != 0 {
                        prev_wide = true;
                        continue;
                    }
                    if !prev_wide {
                        line.push(c.c);
                    }
                    prev_wide = false;
                }
            }
            out.push_str(line.trim_end());
            if y < y1 {
                out.push('\n');
            }
        }
        out
    }
}

/// Paint one full frame.
///
/// `offset` is the number of scrollback rows pinned above the grid
/// (0 = live screen). The cursor is drawn only on the live screen.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    buf: &mut [u32],
    width: usize,
    height: usize,
    term: &Terminal,
    offset: usize,
    palette: &Palette,
    face: &mut FontFace,
    selection: Option<&Selection>,
) {
    // Clear the whole window with the theme background first (also
    // covers gaps when scrolled past the top of the scrollback).
    buf.fill(pack(
        palette.background[0],
        palette.background[1],
        palette.background[2],
    ));

    let cw = face.metrics.cell_width.max(1) as usize;
    let lh = face.metrics.line_height.max(1) as usize;

    let cols = term.cols as usize;
    let rows = term.rows as usize;
    let (cur_x, cur_y) = term.cursor();
    let cursor_on = term.cursor_visible() && offset == 0;

    for y in 0..rows {
        let py = y * lh;
        if py + lh > height {
            break;
        }
        let line = term.visible_row(offset, y as u16);
        for x in 0..cols {
            let px = x * cw;
            if px + cw > width {
                break;
            }
            let cell: Cell = line
                .and_then(|l| l.get(x).copied())
                .unwrap_or_else(Cell::blank);
            if cell.flags & WIDE_SPACER != 0 {
                continue; // second half of a wide glyph
            }

            let mut fg = resolve(cell.fg, palette.foreground, palette);
            let mut bg = resolve(cell.bg, palette.background, palette);
            if cell.flags & INVERSE != 0 {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags & DIM != 0 {
                fg = blend(bg, fg, 0.55);
            }

            let selected = selection.is_some_and(|s| s.contains(x as u16, y as u16));

            // Cursor block: swap the cell to the cursor color and draw
            // the glyph in the background color so it stays readable.
            let on_cursor = cursor_on && cur_x as usize == x && cur_y as usize == y;
            if on_cursor {
                fill_rect(
                    buf,
                    width,
                    height,
                    px as i32,
                    py as i32,
                    cw as u32,
                    lh as u32,
                    pack(palette.cursor[0], palette.cursor[1], palette.cursor[2]),
                );
                fg = palette.background;
            } else if selected {
                let hi = blend(palette.cursor, bg, 0.5);
                fill_rect(
                    buf,
                    width,
                    height,
                    px as i32,
                    py as i32,
                    cw as u32,
                    lh as u32,
                    pack(hi[0], hi[1], hi[2]),
                );
            } else {
                fill_rect(
                    buf,
                    width,
                    height,
                    px as i32,
                    py as i32,
                    cw as u32,
                    lh as u32,
                    pack(bg[0], bg[1], bg[2]),
                );
            }

            if cell.c != ' ' {
                let mut style = 0u8;
                if cell.flags & BOLD != 0 {
                    style |= STYLE_BOLD;
                }
                if cell.flags & ITALIC != 0 {
                    style |= STYLE_ITALIC;
                }
                if let Some(glyph) = face.glyph(cell.c, style) {
                    let gx = px as i32 + glyph.left;
                    let gy = py as i32 + glyph.top;
                    blit_glyph(buf, width, height, glyph, gx, gy, fg, bg);
                }
            }

            if cell.flags & UNDERLINE != 0 {
                draw_underline(
                    buf, width, height, px as i32, py as i32, cw as u32, lh as u32, fg,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontMetrics;

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
        fill_rect(&mut buf, 10, 10, -5, -5, 20, 20, pack(9, 9, 9));
        // Only the visible part is filled.
        assert_eq!(buf[0], pack(9, 9, 9));
        fill_rect(&mut buf, 10, 10, 8, 0, 10, 1, pack(1, 2, 3));
        assert_eq!(buf[9], pack(1, 2, 3));
        // Off-screen rect must not panic.
        fill_rect(&mut buf, 10, 10, 100, 100, 5, 5, 0);
        fill_rect(&mut buf, 10, 10, -1, -1, 0, 0, 0);
    }

    #[test]
    fn fill_rect_fills_rows_not_columns_only() {
        let mut buf = vec![0u32; 4 * 4];
        fill_rect(&mut buf, 4, 4, 1, 1, 2, 2, pack(7, 7, 7));
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
        blit_glyph(&mut buf, 4, 4, &g, 0, 0, [255, 255, 255], [0, 0, 0]);
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
        blit_glyph(&mut buf, 4, 4, &g, 2, 2, [255, 0, 0], [0, 0, 0]);
        assert_eq!(buf[2 * 4 + 2], pack(255, 0, 0));
        assert_eq!(buf[0], 0);
        // Fully off-screen must not panic.
        blit_glyph(&mut buf, 4, 4, &g, 10, 10, [255, 0, 0], [0, 0, 0]);
    }

    #[test]
    fn underline_draws_at_cell_bottom() {
        let mut buf = vec![0u32; 4 * 8];
        draw_underline(&mut buf, 4, 8, 0, 0, 4, 8, [10, 20, 30]);
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
        paint(&mut buf, w, h, &t, 0, &p, &mut face, None);
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
        paint(&mut buf, w, h, &t, 0, &p, &mut face, None);
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
        paint(&mut buf, w, h, &t, 0, &p, &mut face, None);
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
        paint(&mut buf, w, h, &t, 0, &p, &mut face, None);
        // Top-left cell = cursor block -> red.
        assert_eq!(
            unpack(buf[face.metrics.cell_width as usize / 2]),
            [255, 0, 0]
        );
        // Cursor hidden -> no red block.
        t.feed(b"\x1b[?25l");
        let mut buf2 = vec![0u32; w * h];
        paint(&mut buf2, w, h, &t, 0, &p, &mut face, None);
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
        paint(&mut live, w, h, &t, 0, &p, &mut face, None);
        // Offset 2: top visible line is "one".
        let mut scrolled = vec![0u32; w * h];
        paint(&mut scrolled, w, h, &t, 2, &p, &mut face, None);
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
        paint(&mut with_sel, w, h, &t, 0, &p, &mut face, Some(&sel));
        let mut without = vec![0u32; w * h];
        paint(&mut without, w, h, &t, 0, &p, &mut face, None);
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
}
