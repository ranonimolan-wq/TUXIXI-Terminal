//! Renderer core types: the [`Palette`], the [`Selection`] span,
//! the packed-RGB32 pixel helpers and the [`Canvas`] drawing
//! surface whose methods clip and blend every primitive. Shared by
//! the CPU reference painter and the GPU scene builder.

use crate::vt::{Color, Terminal};

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
/// The painter's drawing surface: a packed-RGB32 pixel buffer plus its
/// dimensions. Every primitive (rect fill, glyph blit, underline) is a
/// method, so callers never re-pass buffer + size per call, clipping
/// rules live in exactly one place and no primitive exceeds the
/// argument limit.
pub struct Canvas<'a> {
    pub(crate) buf: &'a mut [u32],
    pub(crate) width: usize,
    pub(crate) height: usize,
}

impl<'a> Canvas<'a> {
    /// Wrap an existing pixel buffer. The buffer must contain at least
    /// `width * height` pixels; callers guarantee this by construction.
    pub fn new(buf: &'a mut [u32], width: usize, height: usize) -> Self {
        Self { buf, width, height }
    }

    /// Fill a rect with a solid packed color, clipped to the buffer.
    /// Off-screen rects are clipped, never wrap around, never panic.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, px: u32) {
        let (width, height) = (self.width, self.height);
        let buf = &mut *self.buf;
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
    /// Coverage 255 replaces the pixel, lower values blend toward it.
    pub fn blit_glyph(
        &mut self,
        glyph: &crate::font::GlyphBitmap,
        dest_x: i32,
        dest_y: i32,
        fg: [u8; 3],
    ) {
        let (width, height) = (self.width, self.height);
        let buf = &mut *self.buf;
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
    }

    /// Draw a horizontal underline at the bottom of a cell.
    pub fn draw_underline(&mut self, x: i32, y: i32, cw: u32, lh: u32, fg: [u8; 3]) {
        let line_y = y + lh as i32 - 2;
        self.fill_rect(x, line_y, cw, 1, pack(fg[0], fg[1], fg[2]));
    }
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
