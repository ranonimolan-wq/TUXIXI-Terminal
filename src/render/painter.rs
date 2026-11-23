//! CPU reference painter: mirrors the GPU scene builder's cell
//! semantics pixel by pixel, so renderer tests can assert on real
//! pixels. Not used by the GUI (which renders via `crate::gpu`) but
//! kept as the behavioral oracle.

use crate::font::{FontFace, STYLE_BOLD, STYLE_ITALIC};
use crate::vt::{Cell, Terminal, BOLD, DIM, INVERSE, ITALIC, UNDERLINE, WIDE_SPACER};

use super::canvas::{blend, pack, resolve, Canvas, Palette, Selection};

/// Paint one full frame.
///
/// `offset` is the number of scrollback rows pinned above the grid
/// (0 = live screen). The cursor is drawn only on the live screen.
pub fn paint(
    canvas: &mut Canvas,
    term: &Terminal,
    offset: usize,
    palette: &Palette,
    face: &mut FontFace,
    selection: Option<&Selection>,
) {
    let (width, height) = (canvas.width, canvas.height);
    // Clear the whole window with the theme background first (also
    // covers gaps when scrolled past the top of the scrollback).
    canvas.fill_rect(
        0,
        0,
        width as u32,
        height as u32,
        pack(
            palette.background[0],
            palette.background[1],
            palette.background[2],
        ),
    );

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
                canvas.fill_rect(
                    px as i32,
                    py as i32,
                    cw as u32,
                    lh as u32,
                    pack(palette.cursor[0], palette.cursor[1], palette.cursor[2]),
                );
                fg = palette.background;
            } else if selected {
                let hi = blend(palette.cursor, bg, 0.5);
                canvas.fill_rect(
                    px as i32,
                    py as i32,
                    cw as u32,
                    lh as u32,
                    pack(hi[0], hi[1], hi[2]),
                );
            } else {
                canvas.fill_rect(
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
                    canvas.blit_glyph(glyph, gx, gy, fg);
                }
            }

            if cell.flags & UNDERLINE != 0 {
                canvas.draw_underline(px as i32, py as i32, cw as u32, lh as u32, fg);
            }
        }
    }
}
