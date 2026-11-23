// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Per-frame scene builder: turns the terminal grid plus palette,
//! selection and glass state into the rectangle and glyph draw lists
//! the renderer consumes. Semantics mirror the classic CPU painter.

use crate::font::{FontFace, STYLE_BOLD, STYLE_ITALIC};
use crate::render::{blend, resolve, Palette, Selection};
use crate::vt::{Cell, Color, Terminal, BOLD, DIM, INVERSE, ITALIC, UNDERLINE, WIDE_SPACER};

use super::atlas::Atlas;
use super::draw::{rgba01, GlassParams, GlyphDraw, RectDraw, Scene};

/// Everything `build_scene` needs besides the terminal grid itself:
/// palette, fonts, glyph atlas, active selection and glass mode.
/// Bundled into one descriptor so the builder keeps a small, stable
/// signature (and gains fields without growing its parameter list).
pub struct SceneInputs<'a> {
    /// Visible grid snapshot (row-major, `cols * rows` cells) produced
    /// by `Terminal::copy_visible` for the requested scroll offset.
    pub grid: &'a [Cell],
    pub palette: &'a Palette,
    pub face: &'a mut FontFace,
    pub atlas: &'a mut Atlas,
    pub selection: Option<&'a Selection>,
    pub glass: GlassParams,
}

/// Build the draw scene for one frame. Pixel geometry uses the font
/// cell metrics (`cw` x `lh`), identical to `render::paint`.
///
/// Semantics mirrored from the CPU reference renderer:
///
/// * the cleared background covers default-bg cells (they get NO rect
///   — with glass this is exactly what the compositor blurs through),
/// * non-default cell backgrounds, cursor and selection are opaque
///   rects,
/// * inverse swaps fg/bg, dim blends fg toward bg, underline is a
///   1-px rect at the cell bottom,
/// * the cursor is only drawn on the live screen (offset == 0).
pub fn build_scene(term: &Terminal, offset: usize, inputs: SceneInputs<'_>) -> Scene {
    let SceneInputs {
        grid,
        palette,
        face,
        atlas,
        selection,
        glass,
    } = inputs;
    let mut scene = Scene::default();
    let cw = face.metrics.cell_width.max(1) as f32;
    let lh = face.metrics.line_height.max(1) as f32;
    let fg_alpha = glass.fg_alpha();

    let cols = term.cols as usize;
    let rows = term.rows as usize;
    let (cur_x, cur_y) = term.cursor();
    let cursor_on = term.cursor_visible() && offset == 0;

    for y in 0..rows {
        let py = y as f32 * lh;
        for x in 0..cols {
            let px = x as f32 * cw;
            let cell: Cell = grid.get(y * cols + x).copied().unwrap_or_else(Cell::blank);
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
            let on_cursor = cursor_on && cur_x as usize == x && cur_y as usize == y;

            if on_cursor {
                // Cursor block: opaque theme-cursor background; the
                // glyph is drawn in the theme background color so it
                // stays readable (same trick as the CPU renderer).
                scene.rects.push(RectDraw {
                    x: px,
                    y: py,
                    w: cw,
                    h: lh,
                    color: rgba01(palette.cursor, 1.0),
                });
                fg = palette.background;
            } else if selected {
                let hi = blend(palette.cursor, bg, 0.5);
                scene.rects.push(RectDraw {
                    x: px,
                    y: py,
                    w: cw,
                    h: lh,
                    color: rgba01(hi, 1.0),
                });
            } else if cell.bg != Color::Default || cell.flags & INVERSE != 0 {
                // Only non-default backgrounds become rects; the clear
                // already painted the theme background. An INVERSE cell
                // with a default background still needs its rect (the
                // swapped foreground becomes its visible background).
                scene.rects.push(RectDraw {
                    x: px,
                    y: py,
                    w: cw,
                    h: lh,
                    color: rgba01(bg, 1.0),
                });
            }

            if cell.c != ' ' {
                let mut style = 0u8;
                if cell.flags & BOLD != 0 {
                    style |= STYLE_BOLD;
                }
                if cell.flags & ITALIC != 0 {
                    style |= STYLE_ITALIC;
                }
                let key = (cell.c, style);
                if !atlas.entries.contains_key(&key) {
                    if let Some(bitmap) = face.glyph(cell.c, style) {
                        let bitmap = bitmap.clone();
                        // Full: reset and repack this glyph once.
                        if !atlas.insert(key, &bitmap) {
                            atlas.reset();
                            atlas.insert(key, &bitmap);
                        }
                    }
                }
                if let Some(entry) = atlas.entry(key) {
                    if let Some(bitmap) = face.glyph(cell.c, style) {
                        scene.glyphs.push(GlyphDraw {
                            x: px + bitmap.left as f32,
                            y: py + bitmap.top as f32,
                            w: bitmap.width as f32,
                            h: bitmap.height as f32,
                            uv: entry.uv,
                            color: rgba01(fg, fg_alpha),
                        });
                    }
                }
            }

            if cell.flags & UNDERLINE != 0 {
                scene.rects.push(RectDraw {
                    x: px,
                    y: py + lh - 1.0,
                    w: cw,
                    h: 1.0,
                    color: rgba01(fg, 1.0),
                });
            }
        }
    }

    scene
}

/// Build the clear color for one frame: the theme background at the
/// glass opacity. This is what the compositor blurs through.
pub fn clear_color(palette: &Palette, glass: GlassParams) -> [f32; 4] {
    rgba01(palette.background, glass.bg_alpha())
}
